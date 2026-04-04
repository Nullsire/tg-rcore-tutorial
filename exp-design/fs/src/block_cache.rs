use crate::disk::{Block, FileDisk, BLOCK_SIZE};
use std::collections::HashMap;

/// A cached block entry.
struct CachedBlock {
    data: Block,
    dirty: bool,
    pinned: bool, // pinned blocks can't be evicted (used by log)
}

/// A simple write-through block cache.
/// Caches disk blocks in memory, tracks dirty state for write-back.
pub struct BlockCache {
    cache: HashMap<u32, CachedBlock>,
}

impl BlockCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Read a block (from cache if present, otherwise from disk).
    pub fn read(&mut self, disk: &mut FileDisk, block_no: u32) -> &Block {
        if !self.cache.contains_key(&block_no) {
            let mut data: Block = [0u8; BLOCK_SIZE];
            disk.read_block(block_no, &mut data)
                .unwrap_or_else(|e| panic!("cache read block {}: {}", block_no, e));
            self.cache.insert(
                block_no,
                CachedBlock {
                    data,
                    dirty: false,
                    pinned: false,
                },
            );
        }
        &self.cache.get(&block_no).unwrap().data
    }

    /// Read a block mutably. Marks it as dirty.
    pub fn read_mut(&mut self, disk: &mut FileDisk, block_no: u32) -> &mut Block {
        if !self.cache.contains_key(&block_no) {
            let mut data: Block = [0u8; BLOCK_SIZE];
            disk.read_block(block_no, &mut data)
                .unwrap_or_else(|e| panic!("cache read block {}: {}", block_no, e));
            self.cache.insert(
                block_no,
                CachedBlock {
                    data,
                    dirty: false,
                    pinned: false,
                },
            );
        }
        let entry = self.cache.get_mut(&block_no).unwrap();
        entry.dirty = true;
        &mut entry.data
    }

    /// Write data to a cached block (does NOT go to disk until sync).
    /// Returns the old content of the block (useful for undo log).
    pub fn write(&mut self, disk: &mut FileDisk, block_no: u32, data: &Block) -> Block {
        let old = if let Some(entry) = self.cache.get_mut(&block_no) {
            let old = entry.data;
            entry.data = *data;
            entry.dirty = true;
            old
        } else {
            let mut old: Block = [0u8; BLOCK_SIZE];
            disk.read_block(block_no, &mut old)
                .unwrap_or_else(|e| panic!("cache write block {}: {}", block_no, e));
            self.cache.insert(
                block_no,
                CachedBlock {
                    data: *data,
                    dirty: true,
                    pinned: false,
                },
            );
            old
        };
        old
    }

    /// Flush all dirty blocks to disk and clear dirty flags.
    /// Pinned blocks are NOT flushed (they are managed by the log).
    pub fn sync(&mut self, disk: &mut FileDisk) {
        for (&block_no, entry) in &mut self.cache.iter_mut() {
            if entry.dirty && !entry.pinned {
                disk.write_block(block_no, &entry.data)
                    .unwrap_or_else(|e| panic!("cache sync block {}: {}", block_no, e));
                entry.dirty = false;
            }
        }
    }

    /// Sync only specific blocks (used by log install).
    pub fn sync_blocks(&mut self, disk: &mut FileDisk, blocks: &[u32]) {
        for &block_no in blocks {
            if let Some(entry) = self.cache.get_mut(&block_no) {
                if entry.dirty {
                    disk.write_block(block_no, &entry.data)
                        .unwrap_or_else(|e| panic!("cache sync_blocks {}: {}", block_no, e));
                    entry.dirty = false;
                }
            }
        }
    }

    /// Pin a block (prevent sync from writing it).
    pub fn pin(&mut self, block_no: u32) {
        if let Some(entry) = self.cache.get_mut(&block_no) {
            entry.pinned = true;
        }
    }

    /// Unpin a block.
    pub fn unpin(&mut self, block_no: u32) {
        if let Some(entry) = self.cache.get_mut(&block_no) {
            entry.pinned = false;
        }
    }

    /// Invalidate a cached block (remove from cache).
    pub fn invalidate(&mut self, block_no: u32) {
        self.cache.remove(&block_no);
    }

    /// Clear the entire cache (simulate cache loss on crash).
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Check if a block is cached.
    pub fn contains(&self, block_no: u32) -> bool {
        self.cache.contains_key(&block_no)
    }

    /// Get the number of cached blocks.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}
