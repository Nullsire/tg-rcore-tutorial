pub mod stdio;
pub mod ramfs;

pub use stdio::{Stdin, Stdout};
pub use ramfs::RamFile;

pub trait File: Send + Sync {
    fn readable(&self) -> bool;
    fn writable(&self) -> bool;
    fn read(&self, buf: &mut [u8]) -> usize;
    fn write(&self, buf: &[u8]) -> usize;
    fn seek(&self, _offset: isize, _whence: usize) -> isize {
        -1
    }
    fn size(&self) -> usize {
        0
    }
}
