use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const BLOCK_SIZE: usize = 512;
pub type Block = [u8; BLOCK_SIZE];

/// A file-backed block device that simulates a physical disk.
pub struct FileDisk {
    file: File,
    pub nblocks: u32,
}

impl FileDisk {
    /// Create a new disk image file of `nblocks` blocks, initialized to zeros.
    pub fn create(path: &Path, nblocks: u32) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len((nblocks as u64) * (BLOCK_SIZE as u64))?;
        Ok(Self { file, nblocks })
    }

    /// Open an existing disk image.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let metadata = file.metadata()?;
        let nblocks = (metadata.len() / BLOCK_SIZE as u64) as u32;
        Ok(Self { file, nblocks })
    }

    /// Read a block from disk into `buf`.
    pub fn read_block(&mut self, block_no: u32, buf: &mut Block) -> std::io::Result<()> {
        assert!(block_no < self.nblocks, "read_block: out of range");
        self.file
            .seek(SeekFrom::Start(block_no as u64 * BLOCK_SIZE as u64))?;
        self.file.read_exact(buf)?;
        Ok(())
    }

    /// Write a block to disk and flush to ensure persistence.
    pub fn write_block(&mut self, block_no: u32, buf: &Block) -> std::io::Result<()> {
        assert!(block_no < self.nblocks, "write_block: out of range");
        self.file
            .seek(SeekFrom::Start(block_no as u64 * BLOCK_SIZE as u64))?;
        self.file.write_all(buf)?;
        self.file.flush()?;
        Ok(())
    }

    /// Simulate a crash: drop without flushing any pending data.
    /// In our design, the cache calls this instead of sync().
    pub fn crash(self) {
        // Just drop the file handle without flushing
        drop(self);
    }

    /// Flush any OS-level buffers to physical storage.
    pub fn sync(&mut self) -> std::io::Result<()> {
        self.file.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_disk_read_write() {
        let path = PathBuf::from("/tmp/jfs_test_disk.img");
        let nblocks = 100;
        {
            let mut disk = FileDisk::create(&path, nblocks).unwrap();

            // Write a block
            let mut data: Block = [0u8; BLOCK_SIZE];
            data[0..5].copy_from_slice(b"hello");
            disk.write_block(10, &data).unwrap();

            // Read it back
            let mut buf: Block = [0u8; BLOCK_SIZE];
            disk.read_block(10, &mut buf).unwrap();
            assert_eq!(&buf[0..5], b"hello");
        }
        std::fs::remove_file(&path).ok();
    }
}
