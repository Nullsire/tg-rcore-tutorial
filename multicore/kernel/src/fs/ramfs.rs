use super::File;
use crate::sync::SpinLock;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub const O_CREATE: usize = 1;
pub const O_TRUNC: usize = 2;

struct Inode {
    data: SpinLock<Vec<u8>>,
}

impl Inode {
    fn new() -> Self {
        Self {
            data: SpinLock::new(Vec::new()),
        }
    }
}

static RAMFS: SpinLock<Option<BTreeMap<String, Arc<Inode>>>> = SpinLock::new(None);

fn with_fs_mut<R>(f: impl FnOnce(&mut BTreeMap<String, Arc<Inode>>) -> R) -> R {
    let mut guard = RAMFS.lock();
    if guard.is_none() {
        *guard = Some(BTreeMap::new());
    }
    let fs = guard.as_mut().unwrap();
    f(fs)
}

fn open_inode(path: &str, flags: usize) -> Option<Arc<Inode>> {
    with_fs_mut(|fs| {
        if let Some(inode) = fs.get(path) {
            if flags & O_TRUNC != 0 {
                inode.data.lock().clear();
            }
            Some(inode.clone())
        } else if flags & O_CREATE != 0 {
            let inode = Arc::new(Inode::new());
            fs.insert(path.into(), inode.clone());
            Some(inode)
        } else {
            None
        }
    })
}

pub fn unlink(path: &str) -> isize {
    with_fs_mut(|fs| if fs.remove(path).is_some() { 0 } else { -1 })
}

pub struct RamFile {
    inode: Arc<Inode>,
    offset: SpinLock<usize>,
}

impl RamFile {
    pub fn open(path: &str, flags: usize) -> Option<Arc<Self>> {
        let inode = open_inode(path, flags)?;
        Some(Arc::new(Self {
            inode,
            offset: SpinLock::new(0),
        }))
    }
}

impl File for RamFile {
    fn readable(&self) -> bool {
        true
    }

    fn writable(&self) -> bool {
        true
    }

    fn read(&self, buf: &mut [u8]) -> usize {
        let data = self.inode.data.lock();
        let mut off = self.offset.lock();
        if *off >= data.len() {
            return 0;
        }
        let end = (*off + buf.len()).min(data.len());
        let n = end - *off;
        buf[..n].copy_from_slice(&data[*off..end]);
        *off = end;
        n
    }

    fn write(&self, buf: &[u8]) -> usize {
        let mut data = self.inode.data.lock();
        let mut off = self.offset.lock();
        let end = *off + buf.len();
        if end > data.len() {
            data.resize(end, 0);
        }
        data[*off..end].copy_from_slice(buf);
        *off = end;
        buf.len()
    }

    fn seek(&self, offset: isize, whence: usize) -> isize {
        let data_len = self.inode.data.lock().len() as isize;
        let mut off = self.offset.lock();
        let base = match whence {
            0 => 0,
            1 => *off as isize,
            2 => data_len,
            _ => return -1,
        };
        let new_off = base + offset;
        if new_off < 0 {
            return -1;
        }
        *off = new_off as usize;
        new_off
    }

    fn size(&self) -> usize {
        self.inode.data.lock().len()
    }
}
