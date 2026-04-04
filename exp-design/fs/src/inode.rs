use crate::block_cache::BlockCache;
use crate::bitmap::Bitmap;
use crate::disk::{FileDisk, BLOCK_SIZE};
use crate::superblock::Superblock;

/// Inode types.
pub const INODE_NONE: u16 = 0;
pub const INODE_FILE: u16 = 1;
pub const INODE_DIR: u16 = 2;

/// Number of direct block pointers per inode.
pub const NDIRECT: usize = 12;

/// On-disk inode size in bytes: 2 + 2 + 4 + 12*4 = 56, padded to 64.
pub const INODE_SIZE: usize = 64;

/// On-disk inode (64 bytes, fits 8 per block).
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct DiskInode {
    pub itype: u16,          // 0=none, 1=file, 2=dir
    pub nlink: u16,          // number of hard links
    pub size: u32,           // file size in bytes
    pub direct: [u32; NDIRECT], // direct block pointers
    // 8 bytes implicit padding to reach 64
}

impl DiskInode {
    pub fn zeroed() -> Self {
        Self {
            itype: INODE_NONE,
            nlink: 0,
            size: 0,
            direct: [0u32; NDIRECT],
        }
    }

    pub fn to_bytes(&self) -> [u8; INODE_SIZE] {
        let mut buf = [0u8; INODE_SIZE];
        buf[0..2].copy_from_slice(&self.itype.to_le_bytes());
        buf[2..4].copy_from_slice(&self.nlink.to_le_bytes());
        buf[4..8].copy_from_slice(&self.size.to_le_bytes());
        for i in 0..NDIRECT {
            let off = 8 + i * 4;
            buf[off..off + 4].copy_from_slice(&self.direct[i].to_le_bytes());
        }
        buf
    }

    pub fn from_bytes(buf: &[u8; INODE_SIZE]) -> Self {
        Self {
            itype: u16::from_le_bytes(buf[0..2].try_into().unwrap()),
            nlink: u16::from_le_bytes(buf[2..4].try_into().unwrap()),
            size: u32::from_le_bytes(buf[4..8].try_into().unwrap()),
            direct: {
                let mut d = [0u32; NDIRECT];
                for i in 0..NDIRECT {
                    let off = 8 + i * 4;
                    d[i] = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
                }
                d
            },
        }
    }
}

/// Number of inodes per block (512 / 32 = 16).
pub const INODES_PER_BLOCK: u32 = (BLOCK_SIZE / INODE_SIZE) as u32;

/// In-memory inode wrapper.
pub struct Inode {
    pub inum: u32,          // inode number
    pub disk: DiskInode,    // on-disk copy
    pub valid: bool,        // has been read from disk
    pub ref_count: u32,     // reference count
}

impl Inode {
    pub fn new(inum: u32) -> Self {
        Self {
            inum,
            disk: DiskInode::zeroed(),
            valid: false,
            ref_count: 0,
        }
    }
}

/// Inode operations.
pub struct InodeOps;

impl InodeOps {
    /// Allocate a new inode of the given type.
    /// Returns the inode number.
    pub fn ialloc(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        itype: u16,
    ) -> Option<u32> {
        for inum in 1..sb.ninodes {
            let block_no = sb.inode_start + inum / INODES_PER_BLOCK;
            let offset = ((inum % INODES_PER_BLOCK) * INODE_SIZE as u32) as usize;

            let data = cache.read_mut(disk, block_no);
            let mut dinode = DiskInode::from_bytes((&data[offset..offset + INODE_SIZE]).try_into().unwrap());

            if dinode.itype == INODE_NONE {
                dinode = DiskInode::zeroed();
                dinode.itype = itype;
                dinode.nlink = 1;
                dinode.size = 0;
                data[offset..offset + INODE_SIZE].copy_from_slice(&dinode.to_bytes());
                return Some(inum);
            }
        }
        None
    }

    /// Read an inode from disk into the Inode struct.
    pub fn iget(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        inum: u32,
    ) -> DiskInode {
        assert!(inum > 0 && inum < sb.ninodes, "iget: invalid inode {}", inum);

        let block_no = sb.inode_start + inum / INODES_PER_BLOCK;
        let offset = ((inum % INODES_PER_BLOCK) * INODE_SIZE as u32) as usize;

        let data = cache.read(disk, block_no);
        DiskInode::from_bytes((&data[offset..offset + INODE_SIZE]).try_into().unwrap())
    }

    /// Write an inode back to disk (through cache).
    pub fn iupdate(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        inum: u32,
        dinode: &DiskInode,
    ) {
        let block_no = sb.inode_start + inum / INODES_PER_BLOCK;
        let offset = ((inum % INODES_PER_BLOCK) * INODE_SIZE as u32) as usize;

        let data = cache.read_mut(disk, block_no);
        data[offset..offset + INODE_SIZE].copy_from_slice(&dinode.to_bytes());
    }

    /// Allocate a data block for an inode.
    /// Returns the block number of the newly allocated block.
    pub fn balloc(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
    ) -> Option<u32> {
        let ndata = sb.ndata_blocks();
        let bnum = Bitmap::alloc(cache, disk, sb.data_bitmap, ndata)?;
        // The actual disk block number
        let block_no = sb.data_start + bnum;
        // Zero the new block
        let data = cache.read_mut(disk, block_no);
        *data = [0u8; BLOCK_SIZE];
        Some(block_no)
    }

    /// Free a data block.
    pub fn bfree(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        block_no: u32,
    ) {
        assert!(block_no >= sb.data_start, "bfree: not a data block");
        let bnum = block_no - sb.data_start;
        Bitmap::free(cache, disk, sb.data_bitmap, bnum);
    }

    /// Read data from an inode. Returns the number of bytes read.
    pub fn readi(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        dinode: &DiskInode,
        buf: &mut [u8],
        offset: u32,
    ) -> usize {
        let size = dinode.size;
        if offset >= size {
            return 0;
        }
        let end = std::cmp::min(offset + buf.len() as u32, size);
        let mut bytes_read = 0usize;
        let mut off = offset;

        while off < end {
            let block_idx = (off / BLOCK_SIZE as u32) as usize;
            let block_off = (off % BLOCK_SIZE as u32) as usize;
            let bytes_left = (end - off) as usize;
            let chunk = std::cmp::min(bytes_left, BLOCK_SIZE - block_off);

            if block_idx >= NDIRECT {
                break;
            }

            let data_block = dinode.direct[block_idx];
            if data_block == 0 {
                // Hole — fill with zeros
                buf[bytes_read..bytes_read + chunk].fill(0);
            } else {
                let data = cache.read(disk, data_block);
                buf[bytes_read..bytes_read + chunk]
                    .copy_from_slice(&data[block_off..block_off + chunk]);
            }

            bytes_read += chunk;
            off += chunk as u32;
        }

        bytes_read
    }

    /// Write data to an inode. Returns the number of bytes written.
    pub fn writei(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        sb: &Superblock,
        dinode: &mut DiskInode,
        inum: u32,
        buf: &[u8],
        offset: u32,
    ) -> usize {
        let mut bytes_written = 0usize;
        let mut off = offset;

        while bytes_written < buf.len() {
            let block_idx = (off / BLOCK_SIZE as u32) as usize;
            let block_off = (off % BLOCK_SIZE as u32) as usize;
            let bytes_left = buf.len() - bytes_written;
            let chunk = std::cmp::min(bytes_left, BLOCK_SIZE - block_off);

            if block_idx >= NDIRECT {
                break;
            }

            // Allocate block if needed
            if dinode.direct[block_idx] == 0 {
                match Self::balloc(cache, disk, sb) {
                    Some(b) => dinode.direct[block_idx] = b,
                    None => break,
                }
            }

            let data_block = dinode.direct[block_idx];
            let data = cache.read_mut(disk, data_block);
            let src_start = bytes_written;
            data[block_off..block_off + chunk]
                .copy_from_slice(&buf[src_start..src_start + chunk]);

            bytes_written += chunk;
            off += chunk as u32;
        }

        // Update inode size
        let new_size = std::cmp::max(dinode.size, offset + bytes_written as u32);
        dinode.size = new_size;

        Self::iupdate(cache, disk, sb, inum, dinode);
        bytes_written
    }
}
