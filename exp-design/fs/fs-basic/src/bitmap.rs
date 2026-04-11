use crate::block_cache::BlockCache;
use crate::disk::{FileDisk, BLOCK_SIZE};

/// Bitmap allocator for inodes and data blocks.
/// Each bit represents one resource (inode or data block).
pub struct Bitmap;

impl Bitmap {
    /// Allocate a new resource from the bitmap.
    /// `bitmap_block` is the block number containing the bitmap.
    /// `max_items` is the total number of items this bitmap tracks.
    /// Returns the index of the allocated item.
    pub fn alloc(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        bitmap_block: u32,
        max_items: u32,
    ) -> Option<u32> {
        let data = cache.read_mut(disk, bitmap_block);
        let bits = max_items.min((BLOCK_SIZE * 8) as u32);

        for i in 0..bits {
            let byte_idx = (i / 8) as usize;
            let bit_idx = (i % 8) as usize;
            if data[byte_idx] & (1 << bit_idx) == 0 {
                data[byte_idx] |= 1 << bit_idx;
                return Some(i);
            }
        }
        None
    }

    /// Free a resource in the bitmap.
    pub fn free(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        bitmap_block: u32,
        index: u32,
    ) {
        let data = cache.read_mut(disk, bitmap_block);
        let byte_idx = (index / 8) as usize;
        let bit_idx = (index % 8) as usize;
        data[byte_idx] &= !(1 << bit_idx);
    }

    /// Check if a resource is allocated.
    pub fn is_allocated(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        bitmap_block: u32,
        index: u32,
    ) -> bool {
        let data = cache.read(disk, bitmap_block);
        let byte_idx = (index / 8) as usize;
        let bit_idx = (index % 8) as usize;
        data[byte_idx] & (1 << bit_idx) != 0
    }

    /// Mark a specific index as allocated.
    pub fn mark(
        cache: &mut BlockCache,
        disk: &mut FileDisk,
        bitmap_block: u32,
        index: u32,
    ) {
        let data = cache.read_mut(disk, bitmap_block);
        let byte_idx = (index / 8) as usize;
        let bit_idx = (index % 8) as usize;
        data[byte_idx] |= 1 << bit_idx;
    }
}

/// Mark all blocks in a range as free (zero the bitmap block).
pub fn bitmap_init(cache: &mut BlockCache, disk: &mut FileDisk, bitmap_block: u32) {
    let data = cache.read_mut(disk, bitmap_block);
    *data = [0u8; BLOCK_SIZE];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::superblock::Superblock;
    use std::path::PathBuf;

    #[test]
    fn test_bitmap_alloc_free() {
        let path = PathBuf::from("/tmp/jfs_test_bitmap.img");
        let nblocks = 1024;
        let mut d = crate::disk::FileDisk::create(&path, nblocks).unwrap();
        let mut cache = BlockCache::new();

        // Use block 10 as bitmap
        bitmap_init(&mut cache, &mut d, 10);

        // Allocate 3 items
        let a = Bitmap::alloc(&mut cache, &mut d, 10, 100).unwrap();
        let b = Bitmap::alloc(&mut cache, &mut d, 10, 100).unwrap();
        let c = Bitmap::alloc(&mut cache, &mut d, 10, 100).unwrap();
        assert_ne!(a, b);
        assert_ne!(b, c);

        // Free one and re-allocate should get the same
        Bitmap::free(&mut cache, &mut d, 10, b);
        let d2 = Bitmap::alloc(&mut cache, &mut d, 10, 100).unwrap();
        assert_eq!(d2, b);

        std::fs::remove_file(&path).ok();
    }
}
