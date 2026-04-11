use crate::disk::{Block, FileDisk, BLOCK_SIZE};

/// Magic number to identify our file system.
pub const FS_MAGIC: u32 = 0x4A465331; // "JFS1"

/// Number of blocks reserved for the log area.
pub const LOG_BLOCKS: u32 = 64;

/// Maximum number of inodes.
pub const NUM_INODES: u32 = 200;

/// On-disk superblock (lives in block 1).
/// Packed into a single 512-byte block.
#[derive(Debug, Clone)]
pub struct Superblock {
    pub magic: u32,
    pub version: u32,
    pub nblocks: u32,        // total number of blocks
    pub nlogblocks: u32,     // number of log blocks
    pub log_start: u32,      // block number where log begins
    pub ninodes: u32,        // number of inodes
    pub inode_start: u32,    // block number where inode blocks begin
    pub inode_bitmap: u32,   // block number of inode bitmap
    pub data_bitmap: u32,    // block number of data bitmap
    pub data_start: u32,     // block number where data blocks begin
}

impl Superblock {
    /// Layout the file system and return a superblock.
    /// Disk layout:
    ///   Block 0: unused (boot block)
    ///   Block 1: superblock
    ///   Block 2..2+nlogblocks: log
    ///   Block next: inode bitmap
    ///   Block next+1: data bitmap
    ///   Block next+2..: inode blocks (each holds 16 inodes of 32 bytes)
    ///   Block ...: data blocks
    pub fn new(nblocks: u32) -> Self {
        assert!(nblocks > 64, "disk too small");

        let log_start: u32 = 2;
        let inode_bitmap: u32 = log_start + LOG_BLOCKS;
        let data_bitmap: u32 = inode_bitmap + 1;

        // Each inode is 64 bytes, so 512/64 = 8 inodes per block
        let inodes_per_block = (BLOCK_SIZE / 64) as u32; // 8
        let ninode_blocks = (NUM_INODES + inodes_per_block - 1) / inodes_per_block;
        let inode_start: u32 = data_bitmap + 1;
        let data_start: u32 = inode_start + ninode_blocks;

        Self {
            magic: FS_MAGIC,
            version: 1,
            nblocks,
            nlogblocks: LOG_BLOCKS,
            log_start,
            ninodes: NUM_INODES,
            inode_start,
            inode_bitmap,
            data_bitmap,
            data_start,
        }
    }

    /// Read superblock from disk (block 1).
    pub fn read_from_disk(disk: &mut FileDisk) -> Self {
        let mut buf: Block = [0u8; BLOCK_SIZE];
        disk.read_block(1, &mut buf).expect("read superblock");
        Self::from_bytes(&buf)
    }

    /// Write superblock to disk (block 1).
    pub fn write_to_disk(&self, disk: &mut FileDisk) {
        let buf = self.to_bytes();
        disk.write_block(1, &buf).expect("write superblock");
    }

    pub fn to_bytes(&self) -> Block {
        let mut buf: Block = [0u8; BLOCK_SIZE];
        let mut off = 0usize;
        buf[off..off + 4].copy_from_slice(&self.magic.to_le_bytes());
        off += 4;
        buf[off..off + 4].copy_from_slice(&self.version.to_le_bytes());
        off += 4;
        buf[off..off + 4].copy_from_slice(&self.nblocks.to_le_bytes());
        off += 4;
        buf[off..off + 4].copy_from_slice(&self.nlogblocks.to_le_bytes());
        off += 4;
        buf[off..off + 4].copy_from_slice(&self.log_start.to_le_bytes());
        off += 4;
        buf[off..off + 4].copy_from_slice(&self.ninodes.to_le_bytes());
        off += 4;
        buf[off..off + 4].copy_from_slice(&self.inode_start.to_le_bytes());
        off += 4;
        buf[off..off + 4].copy_from_slice(&self.inode_bitmap.to_le_bytes());
        off += 4;
        buf[off..off + 4].copy_from_slice(&self.data_bitmap.to_le_bytes());
        off += 4;
        buf[off..off + 4].copy_from_slice(&self.data_start.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &Block) -> Self {
        let magic = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let nblocks = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let nlogblocks = u32::from_le_bytes(buf[12..16].try_into().unwrap());
        let log_start = u32::from_le_bytes(buf[16..20].try_into().unwrap());
        let ninodes = u32::from_le_bytes(buf[20..24].try_into().unwrap());
        let inode_start = u32::from_le_bytes(buf[24..28].try_into().unwrap());
        let inode_bitmap = u32::from_le_bytes(buf[28..32].try_into().unwrap());
        let data_bitmap = u32::from_le_bytes(buf[32..36].try_into().unwrap());
        let data_start = u32::from_le_bytes(buf[36..40].try_into().unwrap());
        Self {
            magic,
            version,
            nblocks,
            nlogblocks,
            log_start,
            ninodes,
            inode_start,
            inode_bitmap,
            data_bitmap,
            data_start,
        }
    }

    /// Number of data blocks available.
    pub fn ndata_blocks(&self) -> u32 {
        self.nblocks - self.data_start
    }

    /// Number of inode blocks.
    pub fn ninode_blocks(&self) -> u32 {
        self.data_start - self.inode_start
    }
}
