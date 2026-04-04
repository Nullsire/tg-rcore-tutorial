use crate::block_cache::BlockCache;
use crate::disk::{Block, FileDisk, BLOCK_SIZE};
use crate::superblock::Superblock;

/// Maximum number of blocks that can be logged in one transaction.
pub const MAX_LOG_BLOCKS: usize = 30;

/// Header block layout:
///   [0..4]   count: u32 — number of logged blocks
///   [4..8]   committed: u32 — 1 if committed, 0 if not
///   [8..8+N*4] block numbers: [u32; N]
///
/// Data blocks follow the header block in the log area.

/// Journal trait — the interface for both redo and undo log.
pub trait Journal {
    /// Start a new transaction.
    fn begin_tx(&mut self);

    /// Record a block write in the current transaction.
    /// `block_no` is the final destination block.
    /// `data` is the content to be written.
    fn log_write(&mut self, block_no: u32, data: &Block);

    /// Commit the current transaction.
    fn commit(&mut self, disk: &mut FileDisk, cache: &mut BlockCache) -> Result<(), String>;

    /// Recover from a crash (called on mount).
    fn recover(&mut self, disk: &mut FileDisk, cache: &mut BlockCache) -> Result<(), String>;

    /// Called when the file system is unmounted — clears the log.
    fn cleanup(&mut self, disk: &mut FileDisk, cache: &mut BlockCache);
}

// ---------------------------------------------------------------------------
// Redo Log (xv6 style)
// ---------------------------------------------------------------------------

/// Redo Log: writes new data to log, then on install copies to final locations.
/// Recovery: if committed, replay logged blocks to final locations.
pub struct RedoLog {
    log_start: u32,
    log_blocks: u32,

    // Transaction state
    in_tx: bool,
    logged_blocks: Vec<(u32, Block)>, // (block_no, data)
}

impl RedoLog {
    pub fn new(sb: &Superblock) -> Self {
        Self {
            log_start: sb.log_start,
            log_blocks: sb.nlogblocks,
            in_tx: false,
            logged_blocks: Vec::new(),
        }
    }

    fn header_block(&self) -> u32 {
        self.log_start
    }

    fn data_block(&self, i: usize) -> u32 {
        self.log_start + 1 + i as u32
    }

    fn read_header(&self, disk: &mut FileDisk) -> LogHeader {
        let mut buf: Block = [0u8; BLOCK_SIZE];
        disk.read_block(self.header_block(), &mut buf).unwrap();
        LogHeader::from_bytes(&buf)
    }

    fn write_header(&self, disk: &mut FileDisk, hdr: &LogHeader) {
        let buf = hdr.to_bytes();
        disk.write_block(self.header_block(), &buf).unwrap();
    }
}

impl Journal for RedoLog {
    fn begin_tx(&mut self) {
        assert!(!self.in_tx, "nested transaction not supported");
        self.in_tx = true;
        self.logged_blocks.clear();
    }

    fn log_write(&mut self, block_no: u32, data: &Block) {
        assert!(self.in_tx, "no active transaction");

        // If we've already logged this block, update in place
        if let Some(entry) = self.logged_blocks.iter_mut().find(|(b, _)| *b == block_no) {
            entry.1 = *data;
            return;
        }

        assert!(
            self.logged_blocks.len() < MAX_LOG_BLOCKS,
            "log transaction too large"
        );
        assert!(
            (self.logged_blocks.len() as u32) < self.log_blocks - 1,
            "log area full"
        );

        self.logged_blocks.push((block_no, *data));
    }

    fn commit(&mut self, disk: &mut FileDisk, cache: &mut BlockCache) -> Result<(), String> {
        assert!(self.in_tx, "no active transaction");

        if self.logged_blocks.is_empty() {
            self.in_tx = false;
            return Ok(());
        }

        // Phase 1: Write logged data blocks to log area
        for (i, (_, data)) in self.logged_blocks.iter().enumerate() {
            disk.write_block(self.data_block(i), data)
                .map_err(|e| format!("log write data: {}", e))?;
        }

        // Phase 2: Write commit header (this is the commit point)
        let mut hdr = LogHeader::new();
        hdr.count = self.logged_blocks.len() as u32;
        hdr.committed = 1;
        for (i, (block_no, _)) in self.logged_blocks.iter().enumerate() {
            hdr.blocks[i] = *block_no;
        }
        self.write_header(disk, &hdr);

        // Phase 3: Install — copy log data to final locations on disk
        for (i, (block_no, _)) in self.logged_blocks.iter().enumerate() {
            let mut data: Block = [0u8; BLOCK_SIZE];
            disk.read_block(self.data_block(i), &mut data)
                .map_err(|e| format!("log read data: {}", e))?;
            disk.write_block(*block_no, &data)
                .map_err(|e| format!("log install: {}", e))?;
        }

        // Phase 4: Clear log header
        let empty_hdr = LogHeader::new();
        self.write_header(disk, &empty_hdr);

        // Update cache with new data
        for (block_no, data) in &self.logged_blocks {
            let _ = cache.write(disk, *block_no, data);
        }

        self.in_tx = false;
        self.logged_blocks.clear();
        Ok(())
    }

    fn recover(&mut self, disk: &mut FileDisk, _cache: &mut BlockCache) -> Result<(), String> {
        let hdr = self.read_header(disk);

        if hdr.count == 0 || hdr.committed == 0 {
            // Nothing to recover
            println!(
                "[RedoLog] No committed transaction to recover (count={}, committed={})",
                hdr.count, hdr.committed
            );

            // Clear any partial log
            let empty = LogHeader::new();
            self.write_header(disk, &empty);
            return Ok(());
        }

        println!(
            "[RedoLog] Recovering {} committed blocks...",
            hdr.count
        );

        // Replay: copy log data to final locations
        for i in 0..hdr.count as usize {
            let mut data: Block = [0u8; BLOCK_SIZE];
            disk.read_block(self.data_block(i), &mut data)
                .map_err(|e| format!("recover read log {}: {}", i, e))?;
            disk.write_block(hdr.blocks[i], &data)
                .map_err(|e| format!("recover write block {}: {}", hdr.blocks[i], e))?;
            println!(
                "  [RedoLog] Replayed log block {} -> disk block {}",
                i, hdr.blocks[i]
            );
        }

        // Clear log header
        let empty = LogHeader::new();
        self.write_header(disk, &empty);

        println!("[RedoLog] Recovery complete.");
        Ok(())
    }

    fn cleanup(&mut self, disk: &mut FileDisk, _cache: &mut BlockCache) {
        if self.in_tx {
            // Abort uncommitted transaction
            self.in_tx = false;
            self.logged_blocks.clear();
        }
        // Clear log header
        let empty = LogHeader::new();
        self.write_header(disk, &empty);
    }
}

// ---------------------------------------------------------------------------
// Undo Log
// ---------------------------------------------------------------------------

/// Undo Log: saves original data to log before overwriting.
/// On commit: writes new data to final locations.
/// Recovery: if committed, restore original data from log (undo the partial writes).
pub struct UndoLog {
    log_start: u32,
    log_blocks: u32,

    // Transaction state
    in_tx: bool,
    /// Maps block_no -> original data (saved before overwrite)
    original_blocks: Vec<(u32, Block)>,
    /// Maps block_no -> new data (to be written on commit)
    new_blocks: Vec<(u32, Block)>,
}

impl UndoLog {
    pub fn new(sb: &Superblock) -> Self {
        Self {
            log_start: sb.log_start,
            log_blocks: sb.nlogblocks,
            in_tx: false,
            original_blocks: Vec::new(),
            new_blocks: Vec::new(),
        }
    }

    fn header_block(&self) -> u32 {
        self.log_start
    }

    fn data_block(&self, i: usize) -> u32 {
        self.log_start + 1 + i as u32
    }

    fn read_header(&self, disk: &mut FileDisk) -> LogHeader {
        let mut buf: Block = [0u8; BLOCK_SIZE];
        disk.read_block(self.header_block(), &mut buf).unwrap();
        LogHeader::from_bytes(&buf)
    }

    fn write_header(&self, disk: &mut FileDisk, hdr: &LogHeader) {
        let buf = hdr.to_bytes();
        disk.write_block(self.header_block(), &buf).unwrap();
    }
}

impl Journal for UndoLog {
    fn begin_tx(&mut self) {
        assert!(!self.in_tx, "nested transaction not supported");
        self.in_tx = true;
        self.original_blocks.clear();
        self.new_blocks.clear();
    }

    fn log_write(&mut self, block_no: u32, data: &Block) {
        assert!(self.in_tx, "no active transaction");

        // Save original if this is the first write to this block
        let already_saved = self.original_blocks.iter().any(|(b, _)| *b == block_no);

        // If we've already queued a write for this block, update it
        if let Some(entry) = self.new_blocks.iter_mut().find(|(b, _)| *b == block_no) {
            entry.1 = *data;
            return;
        }

        assert!(
            self.new_blocks.len() < MAX_LOG_BLOCKS,
            "log transaction too large"
        );

        // Save original data flag (actual saving happens in commit)
        if !already_saved {
            self.original_blocks
                .push((block_no, [0u8; BLOCK_SIZE])); // placeholder
        }

        self.new_blocks.push((block_no, *data));
    }

    fn commit(&mut self, disk: &mut FileDisk, cache: &mut BlockCache) -> Result<(), String> {
        assert!(self.in_tx, "no active transaction");

        if self.new_blocks.is_empty() {
            self.in_tx = false;
            return Ok(());
        }

        // Phase 1: Save original block contents to log
        // We need to deduplicate — only save originals for unique blocks
        let mut unique_origins: Vec<(u32, Block)> = Vec::new();
        for (block_no, _) in &self.new_blocks {
            if !unique_origins.iter().any(|(b, _)| b == block_no) {
                let mut orig: Block = [0u8; BLOCK_SIZE];
                disk.read_block(*block_no, &mut orig)
                    .map_err(|e| format!("undo log read orig {}: {}", block_no, e))?;
                unique_origins.push((*block_no, orig));
            }
        }

        // Write originals to log data blocks
        for (i, (_, data)) in unique_origins.iter().enumerate() {
            disk.write_block(self.data_block(i), data)
                .map_err(|e| format!("undo log write orig: {}", e))?;
        }

        // Phase 2: Write commit header (commit point)
        let mut hdr = LogHeader::new();
        hdr.count = unique_origins.len() as u32;
        hdr.committed = 1;
        for (i, (block_no, _)) in unique_origins.iter().enumerate() {
            hdr.blocks[i] = *block_no;
        }
        self.write_header(disk, &hdr);

        // Phase 3: Write new data to final locations
        for (block_no, data) in &self.new_blocks {
            disk.write_block(*block_no, data)
                .map_err(|e| format!("undo log write final {}: {}", block_no, e))?;
        }

        // Phase 4: Clear log header (all done)
        let empty = LogHeader::new();
        self.write_header(disk, &empty);

        // Update cache
        for (block_no, data) in &self.new_blocks {
            let _ = cache.write(disk, *block_no, data);
        }

        self.in_tx = false;
        self.original_blocks.clear();
        self.new_blocks.clear();
        Ok(())
    }

    fn recover(&mut self, disk: &mut FileDisk, _cache: &mut BlockCache) -> Result<(), String> {
        let hdr = self.read_header(disk);

        if hdr.count == 0 || hdr.committed == 0 {
            println!(
                "[UndoLog] No committed transaction to recover (count={}, committed={})",
                hdr.count, hdr.committed
            );
            let empty = LogHeader::new();
            self.write_header(disk, &empty);
            return Ok(());
        }

        println!(
            "[UndoLog] Recovering: undoing {} partially written blocks...",
            hdr.count
        );

        // Restore original data from log
        for i in 0..hdr.count as usize {
            let mut data: Block = [0u8; BLOCK_SIZE];
            disk.read_block(self.data_block(i), &mut data)
                .map_err(|e| format!("undo recover read {}: {}", i, e))?;
            disk.write_block(hdr.blocks[i], &data)
                .map_err(|e| format!("undo recover write {}: {}", hdr.blocks[i], e))?;
            println!(
                "  [UndoLog] Restored block {} from log",
                hdr.blocks[i]
            );
        }

        // Clear log header
        let empty = LogHeader::new();
        self.write_header(disk, &empty);

        println!("[UndoLog] Recovery complete (undid partial writes).");
        Ok(())
    }

    fn cleanup(&mut self, disk: &mut FileDisk, _cache: &mut BlockCache) {
        if self.in_tx {
            self.in_tx = false;
            self.original_blocks.clear();
            self.new_blocks.clear();
        }
        let empty = LogHeader::new();
        self.write_header(disk, &empty);
    }
}

// ---------------------------------------------------------------------------
// Log Header
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct LogHeader {
    count: u32,
    committed: u32,
    blocks: [u32; MAX_LOG_BLOCKS],
}

impl LogHeader {
    fn new() -> Self {
        Self {
            count: 0,
            committed: 0,
            blocks: [0u32; MAX_LOG_BLOCKS],
        }
    }

    fn to_bytes(&self) -> Block {
        let mut buf: Block = [0u8; BLOCK_SIZE];
        buf[0..4].copy_from_slice(&self.count.to_le_bytes());
        buf[4..8].copy_from_slice(&self.committed.to_le_bytes());
        for i in 0..MAX_LOG_BLOCKS {
            let off = 8 + i * 4;
            buf[off..off + 4].copy_from_slice(&self.blocks[i].to_le_bytes());
        }
        buf
    }

    fn from_bytes(buf: &Block) -> Self {
        let count = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let committed = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let mut blocks = [0u32; MAX_LOG_BLOCKS];
        for i in 0..MAX_LOG_BLOCKS {
            let off = 8 + i * 4;
            blocks[i] = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
        }
        Self {
            count,
            committed,
            blocks,
        }
    }
}

// ---------------------------------------------------------------------------
// NoOp Journal (for comparison — no journaling)
// ---------------------------------------------------------------------------

/// A "journal" that does nothing — used to show what happens without journaling.
pub struct NoOpLog;

impl Journal for NoOpLog {
    fn begin_tx(&mut self) {}

    fn log_write(&mut self, _block_no: u32, _data: &Block) {}

    fn commit(
        &mut self,
        _disk: &mut FileDisk,
        _cache: &mut BlockCache,
    ) -> Result<(), String> {
        Ok(())
    }

    fn recover(
        &mut self,
        _disk: &mut FileDisk,
        _cache: &mut BlockCache,
    ) -> Result<(), String> {
        println!("[NoOpLog] No recovery (no journaling).");
        Ok(())
    }

    fn cleanup(&mut self, _disk: &mut FileDisk, _cache: &mut BlockCache) {}
}
