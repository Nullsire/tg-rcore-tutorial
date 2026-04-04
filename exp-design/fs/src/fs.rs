use crate::block_cache::BlockCache;
use crate::bitmap::Bitmap;
use crate::dir::DirOps;
use crate::disk::{FileDisk, BLOCK_SIZE};
use crate::inode::InodeOps;
use crate::log::{Journal, NoOpLog, RedoLog, UndoLog};
use crate::superblock::Superblock;

/// Journal type selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JournalType {
    Redo,   // xv6-style write-ahead redo log
    Undo,   // undo log
    NoOp,   // no journaling (for comparison)
}

impl std::fmt::Display for JournalType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            JournalType::Redo => write!(f, "redo"),
            JournalType::Undo => write!(f, "undo"),
            JournalType::NoOp => write!(f, "noop"),
        }
    }
}

/// The file system: ties together disk, cache, journal, and superblock.
pub struct FileSystem {
    pub disk: FileDisk,
    pub cache: BlockCache,
    pub sb: Superblock,
    pub journal_type: JournalType,
}

impl FileSystem {
    /// Format a new file system image.
    pub fn format(path: &std::path::Path, nblocks: u32) -> std::io::Result<()> {
        println!("Formatting {} block disk image...", nblocks);

        // Create disk
        let mut disk = FileDisk::create(path, nblocks)?;

        // Initialize superblock
        let sb = Superblock::new(nblocks);

        // Zero the disk
        let zero_block: [u8; BLOCK_SIZE] = [0u8; BLOCK_SIZE];
        for i in 0..nblocks {
            disk.write_block(i, &zero_block)?;
        }

        // Write superblock
        sb.write_to_disk(&mut disk);

        // Initialize block cache
        let mut cache = BlockCache::new();

        // Mark inode 0 as used (unused sentinel)
        Bitmap::mark(&mut cache, &mut disk, sb.inode_bitmap, 0);

        // Allocate root inode
        let root_inum = InodeOps::ialloc(&mut cache, &mut disk, &sb, crate::inode::INODE_DIR)
            .expect("format: failed to alloc root inode");
        assert_eq!(root_inum, 1, "root inode should be 1");

        // Initialize root directory
        DirOps::init_root(&mut cache, &mut disk, &sb);

        // Flush cache
        cache.sync(&mut disk);

        println!("File system formatted successfully.");
        println!("  Total blocks: {}", nblocks);
        println!("  Log blocks:   {} (starting at {})", sb.nlogblocks, sb.log_start);
        println!("  Inodes:       {} (starting at {})", sb.ninodes, sb.inode_start);
        println!("  Data blocks:  {} (starting at {})", sb.ndata_blocks(), sb.data_start);

        Ok(())
    }

    /// Mount an existing file system image.
    pub fn mount(path: &std::path::Path, jtype: JournalType) -> Result<Self, String> {
        let mut disk = FileDisk::open(path).map_err(|e| format!("mount: {}", e))?;
        let mut cache = BlockCache::new();
        let sb = Superblock::read_from_disk(&mut disk);

        if sb.magic != crate::superblock::FS_MAGIC {
            return Err("mount: invalid file system (bad magic)".to_string());
        }

        println!("Mounted file system (journal={})", jtype);

        // Run recovery
        match jtype {
            JournalType::Redo => {
                let mut log = RedoLog::new(&sb);
                log.recover(&mut disk, &mut cache)?;
            }
            JournalType::Undo => {
                let mut log = UndoLog::new(&sb);
                log.recover(&mut disk, &mut cache)?;
            }
            JournalType::NoOp => {
                let mut log = NoOpLog;
                log.recover(&mut disk, &mut cache)?;
            }
        }

        Ok(Self {
            disk,
            cache,
            sb,
            journal_type: jtype,
        })
    }

    /// Create a journal instance of the configured type.
    pub fn make_journal(&self) -> Box<dyn Journal> {
        match self.journal_type {
            JournalType::Redo => Box::new(RedoLog::new(&self.sb)),
            JournalType::Undo => Box::new(UndoLog::new(&self.sb)),
            JournalType::NoOp => Box::new(NoOpLog),
        }
    }

    /// Unmount: sync everything and clear the log.
    pub fn umount(&mut self) {
        // Create journal just for cleanup
        let mut journal = self.make_journal();
        journal.cleanup(&mut self.disk, &mut self.cache);

        // Sync all cached blocks
        self.cache.sync(&mut self.disk);

        // Sync disk
        self.disk.sync().expect("umount sync");

        println!("File system unmounted.");
    }

    /// Print file system statistics.
    pub fn stats(&mut self) {
        println!("=== File System Stats ===");
        println!("Magic:     0x{:08X}", self.sb.magic);
        println!("Version:   {}", self.sb.version);
        println!("Blocks:    {} total, {} data", self.sb.nblocks, self.sb.ndata_blocks());
        println!("Inodes:    {}", self.sb.ninodes);
        println!("Journal:   {}", self.journal_type);

        // List root directory
        let entries = DirOps::list(&mut self.cache, &mut self.disk, &self.sb, crate::dir::ROOT_INUM);
        println!("\nRoot directory entries:");
        for (name, inum, itype) in &entries {
            let type_str = match *itype {
                1 => "FILE",
                2 => "DIR ",
                _ => "NONE",
            };
            println!("  {} (inum={}, type={})", name, inum, type_str);
        }
        println!("========================");
    }
}
