use crate::block_cache::BlockCache;
use crate::disk::{FileDisk, BLOCK_SIZE};
use crate::inode::{DiskInode, InodeOps, INODE_DIR};
use crate::superblock::Superblock;

/// Directory entry: 32 bytes each, 16 entries per block.
/// [0..4]   inode number (u32)
/// [4..32]  name ([u8; 28])
pub const DIRSIZ: usize = 28;
pub const DIRENT_SIZE: usize = 32;
pub const DIRENTS_PER_BLOCK: usize = BLOCK_SIZE / DIRENT_SIZE; // 16

#[derive(Debug, Clone, Copy)]
pub struct DirEntry {
    pub inum: u32,
    pub name: [u8; DIRSIZ],
}

impl DirEntry {
    pub fn empty() -> Self {
        Self {
            inum: 0,
            name: [0u8; DIRSIZ],
        }
    }

    pub fn new(inum: u32, name: &str) -> Self {
        let mut entry = Self::empty();
        entry.inum = inum;
        let bytes = name.as_bytes();
        let len = std::cmp::min(bytes.len(), DIRSIZ);
        entry.name[..len].copy_from_slice(&bytes[..len]);
        entry
    }

    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(DIRSIZ);
        std::str::from_utf8(&self.name[..end]).unwrap_or("")
    }

    pub fn to_bytes(&self) -> [u8; DIRENT_SIZE] {
        let mut buf = [0u8; DIRENT_SIZE];
        buf[0..4].copy_from_slice(&self.inum.to_le_bytes());
        buf[4..4 + DIRSIZ].copy_from_slice(&self.name);
        buf
    }

    pub fn from_bytes(buf: &[u8; DIRENT_SIZE]) -> Self {
        Self {
            inum: u32::from_le_bytes(buf[0..4].try_into().unwrap()),
            name: {
                let mut name = [0u8; DIRSIZ];
                name.copy_from_slice(&buf[4..4 + DIRSIZ]);
                name
            },
        }
    }
}

/// Root inode number.
pub const ROOT_INUM: u32 = 1;

/// Directory operations.
pub struct DirOps;

impl DirOps {
    /// Look up a directory entry by name. Returns the inode number.
    pub fn lookup(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        dir_inum: u32,
        name: &str,
    ) -> Option<u32> {
        let dinode = InodeOps::iget(cache, disk, sb, dir_inum);
        if dinode.itype != INODE_DIR {
            return None;
        }

        let name_bytes = name.as_bytes();
        let len = std::cmp::min(name_bytes.len(), DIRSIZ);

        for block_idx in 0..dinode.size as usize / BLOCK_SIZE {
            let data_block = dinode.direct[block_idx];
            if data_block == 0 {
                continue;
            }
            let data = cache.read(disk, data_block);
            for i in 0..DIRENTS_PER_BLOCK {
                let off = i * DIRENT_SIZE;
                let entry = DirEntry::from_bytes((&data[off..off + DIRENT_SIZE]).try_into().unwrap());
                if entry.inum == 0 {
                    continue;
                }
                if entry.name[..len] == name_bytes[..len]
                    && (len == DIRSIZ || entry.name[len] == 0)
                {
                    return Some(entry.inum);
                }
            }
        }
        None
    }

    /// Link a new entry into a directory.
    pub fn link(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        dir_inum: u32,
        name: &str,
        child_inum: u32,
    ) -> Result<(), String> {
        // Check if name already exists
        if Self::lookup(cache, disk, sb, dir_inum, name).is_some() {
            return Err(format!("link: {} already exists", name));
        }

        let mut dinode = InodeOps::iget(cache, disk, sb, dir_inum);
        if dinode.itype != INODE_DIR {
            return Err("link: not a directory".to_string());
        }

        let new_entry = DirEntry::new(child_inum, name);

        // Find a free slot or append
        for block_idx in 0..crate::inode::NDIRECT {
            if dinode.direct[block_idx] == 0 {
                // Allocate a new block for directory
                let new_block = InodeOps::balloc(cache, disk, sb)
                    .ok_or("link: no free blocks")?;
                dinode.direct[block_idx] = new_block;

                // Update dir size
                dinode.size = ((block_idx + 1) * BLOCK_SIZE) as u32;
                InodeOps::iupdate(cache, disk, sb, dir_inum, &dinode);

                // Write the new entry at the beginning of the new block
                let data = cache.read_mut(disk, new_block);
                data[0..DIRENT_SIZE].copy_from_slice(&new_entry.to_bytes());
                return Ok(());
            }

            let data = cache.read(disk, dinode.direct[block_idx]);
            for i in 0..DIRENTS_PER_BLOCK {
                let off = i * DIRENT_SIZE;
                let entry = DirEntry::from_bytes((&data[off..off + DIRENT_SIZE]).try_into().unwrap());
                if entry.inum == 0 {
                    // Found a free slot
                    let data = cache.read_mut(disk, dinode.direct[block_idx]);
                    data[off..off + DIRENT_SIZE].copy_from_slice(&new_entry.to_bytes());
                    return Ok(());
                }
            }
        }

        Err("link: directory full".to_string())
    }

    /// Unlink an entry from a directory.
    pub fn unlink(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        dir_inum: u32,
        name: &str,
    ) -> Result<u32, String> {
        let dinode = InodeOps::iget(cache, disk, sb, dir_inum);
        if dinode.itype != INODE_DIR {
            return Err("unlink: not a directory".to_string());
        }

        let name_bytes = name.as_bytes();
        let len = std::cmp::min(name_bytes.len(), DIRSIZ);

        for block_idx in 0..crate::inode::NDIRECT {
            let data_block = dinode.direct[block_idx];
            if data_block == 0 {
                continue;
            }
            let data = cache.read(disk, data_block);
            for i in 0..DIRENTS_PER_BLOCK {
                let off = i * DIRENT_SIZE;
                let entry = DirEntry::from_bytes((&data[off..off + DIRENT_SIZE]).try_into().unwrap());
                if entry.inum == 0 {
                    continue;
                }
                if entry.name[..len] == name_bytes[..len]
                    && (len == DIRSIZ || entry.name[len] == 0)
                {
                    let freed_inum = entry.inum;
                    // Clear the entry
                    let data = cache.read_mut(disk, data_block);
                    data[off..off + DIRENT_SIZE].copy_from_slice(&DirEntry::empty().to_bytes());

                    // Decrement nlink
                    let mut child = InodeOps::iget(cache, disk, sb, freed_inum);
                    child.nlink = child.nlink.saturating_sub(1);
                    if child.nlink == 0 {
                        // Free data blocks
                        for &b in &child.direct {
                            if b != 0 {
                                InodeOps::bfree(cache, disk, sb, b);
                            }
                        }
                        child = DiskInode::zeroed();
                    }
                    InodeOps::iupdate(cache, disk, sb, freed_inum, &child);

                    return Ok(freed_inum);
                }
            }
        }

        Err(format!("unlink: {} not found", name))
    }

    /// List directory entries.
    pub fn list(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        dir_inum: u32,
    ) -> Vec<(String, u32, u16)> {
        let mut entries = Vec::new();
        let dinode = InodeOps::iget(cache, disk, sb, dir_inum);
        if dinode.itype != INODE_DIR {
            return entries;
        }

        // First pass: collect (name, inum) pairs
        let mut raw: Vec<(String, u32)> = Vec::new();
        for block_idx in 0..crate::inode::NDIRECT {
            let data_block = dinode.direct[block_idx];
            if data_block == 0 {
                continue;
            }
            let data = cache.read(disk, data_block);
            for i in 0..DIRENTS_PER_BLOCK {
                let off = i * DIRENT_SIZE;
                let entry = DirEntry::from_bytes((&data[off..off + DIRENT_SIZE]).try_into().unwrap());
                if entry.inum != 0 {
                    raw.push((entry.name_str().to_string(), entry.inum));
                }
            }
        }
        // Second pass: look up inode types
        for (name, inum) in raw {
            let child = InodeOps::iget(cache, disk, sb, inum);
            entries.push((name, inum, child.itype));
        }
        entries
    }

    /// Initialize the root directory.
    pub fn init_root(cache: &mut BlockCache, disk: &mut FileDisk, sb: &Superblock) {
        // Root inode should already be allocated as INODE_DIR
        let mut dinode = InodeOps::iget(cache, disk, sb, ROOT_INUM);
        dinode.itype = INODE_DIR;
        dinode.nlink = 1;
        dinode.size = 0;

        // Allocate a data block for root dir
        let block = InodeOps::balloc(cache, disk, sb).expect("init_root: no free blocks");
        dinode.direct[0] = block;
        dinode.size = BLOCK_SIZE as u32;

        // Add "." and ".." entries
        let data = cache.read_mut(disk, block);
        let dot = DirEntry::new(ROOT_INUM, ".");
        let dotdot = DirEntry::new(ROOT_INUM, "..");
        data[0..DIRENT_SIZE].copy_from_slice(&dot.to_bytes());
        data[DIRENT_SIZE..2 * DIRENT_SIZE].copy_from_slice(&dotdot.to_bytes());

        InodeOps::iupdate(cache, disk, sb, ROOT_INUM, &dinode);
    }
}
