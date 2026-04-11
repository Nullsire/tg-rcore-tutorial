use crate::block_cache::BlockCache;
use crate::disk::FileDisk;
use crate::dir::DirOps;
use crate::inode::{InodeOps, INODE_FILE};
use crate::log::Journal;
use crate::superblock::Superblock;

/// High-level file operations that wrap journaling.
pub struct FileOps;

impl FileOps {
    /// Create a new file in the root directory.
    /// Returns the new inode number.
    pub fn create(
        journal: &mut dyn Journal,
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        name: &str,
    ) -> Result<u32, String> {
        journal.begin_tx();

        // Allocate inode
        let inum = InodeOps::ialloc(cache, disk, sb, INODE_FILE)
            .ok_or("create: no free inodes")?;

        // Initialize inode data
        let mut dinode = crate::inode::DiskInode::zeroed();
        dinode.itype = INODE_FILE;
        dinode.nlink = 1;
        dinode.size = 0;
        InodeOps::iupdate(cache, disk, sb, inum, &dinode);

        // Link into root directory
        DirOps::link(cache, disk, sb, crate::dir::ROOT_INUM, name, inum)?;

        journal.commit(disk, cache)?;

        Ok(inum)
    }

    /// Write data to a file.
    pub fn write(
        journal: &mut dyn Journal,
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        name: &str,
        data: &[u8],
    ) -> Result<usize, String> {
        let inum = DirOps::lookup(cache, disk, sb, crate::dir::ROOT_INUM, name)
            .ok_or_else(|| format!("write: {} not found", name))?;

        journal.begin_tx();

        let mut dinode = InodeOps::iget(cache, disk, sb, inum);
        let n = InodeOps::writei(cache, disk, sb, &mut dinode, inum, data, 0);

        journal.commit(disk, cache)?;

        Ok(n)
    }

    /// Append data to a file.
    pub fn append(
        journal: &mut dyn Journal,
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        name: &str,
        data: &[u8],
    ) -> Result<usize, String> {
        let inum = DirOps::lookup(cache, disk, sb, crate::dir::ROOT_INUM, name)
            .ok_or_else(|| format!("append: {} not found", name))?;

        journal.begin_tx();

        let mut dinode = InodeOps::iget(cache, disk, sb, inum);
        let offset = dinode.size;
        let n = InodeOps::writei(cache, disk, sb, &mut dinode, inum, data, offset);

        journal.commit(disk, cache)?;

        Ok(n)
    }

    /// Read data from a file.
    pub fn read(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        name: &str,
        buf: &mut [u8],
    ) -> Result<usize, String> {
        let inum = DirOps::lookup(cache, disk, sb, crate::dir::ROOT_INUM, name)
            .ok_or_else(|| format!("read: {} not found", name))?;

        let dinode = InodeOps::iget(cache, disk, sb, inum);
        Ok(InodeOps::readi(cache, disk, &dinode, buf, 0))
    }

    /// Read entire file as a String.
    pub fn read_string(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        name: &str,
    ) -> Result<String, String> {
        let inum = DirOps::lookup(cache, disk, sb, crate::dir::ROOT_INUM, name)
            .ok_or_else(|| format!("read: {} not found", name))?;

        let dinode = InodeOps::iget(cache, disk, sb, inum);
        let mut buf = vec![0u8; dinode.size as usize];
        let n = InodeOps::readi(cache, disk, &dinode, &mut buf, 0);
        buf.truncate(n);
        String::from_utf8(buf).map_err(|e| format!("utf8 error: {}", e))
    }

    /// Get file size.
    pub fn size(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        name: &str,
    ) -> Result<u32, String> {
        let inum = DirOps::lookup(cache, disk, sb, crate::dir::ROOT_INUM, name)
            .ok_or_else(|| format!("size: {} not found", name))?;
        let dinode = InodeOps::iget(cache, disk, sb, inum);
        Ok(dinode.size)
    }

    /// Delete a file.
    pub fn unlink(
        journal: &mut dyn Journal,
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        name: &str,
    ) -> Result<(), String> {
        journal.begin_tx();
        DirOps::unlink(cache, disk, sb, crate::dir::ROOT_INUM, name)?;
        journal.commit(disk, cache)?;
        Ok(())
    }

    /// Rename a file.
    pub fn rename(
        journal: &mut dyn Journal,
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        old_name: &str,
        new_name: &str,
    ) -> Result<(), String> {
        let inum = DirOps::lookup(cache, disk, sb, crate::dir::ROOT_INUM, old_name)
            .ok_or_else(|| format!("rename: {} not found", old_name))?;

        journal.begin_tx();

        // Remove old name
        DirOps::unlink(cache, disk, sb, crate::dir::ROOT_INUM, old_name)?;

        // Add new name (increment nlink since unlink decremented it)
        let mut dinode = InodeOps::iget(cache, disk, sb, inum);
        dinode.nlink += 1;
        InodeOps::iupdate(cache, disk, sb, inum, &dinode);

        DirOps::link(cache, disk, sb, crate::dir::ROOT_INUM, new_name, inum)?;

        journal.commit(disk, cache)?;
        Ok(())
    }
}
