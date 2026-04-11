use super::SpinLock;
use alloc::collections::VecDeque;

/// Simple blocking mutex that puts threads to sleep
pub struct Mutex {
    inner: SpinLock<MutexInner>,
}

struct MutexInner {
    locked: bool,
    wait_queue: VecDeque<usize>, // thread IDs waiting
}

impl Mutex {
    pub fn new() -> Self {
        Mutex {
            inner: SpinLock::new(MutexInner {
                locked: false,
                wait_queue: VecDeque::new(),
            }),
        }
    }

    pub fn lock(&self) -> bool {
        let mut inner = self.inner.lock();
        if !inner.locked {
            inner.locked = true;
            true
        } else {
            false
        }
    }

    pub fn unlock(&self) -> Option<usize> {
        let mut inner = self.inner.lock();
        if let Some(tid) = inner.wait_queue.pop_front() {
            // Transfer lock to the next waiter
            Some(tid)
        } else {
            inner.locked = false;
            None
        }
    }

    pub fn add_waiter(&self, tid: usize) {
        let mut inner = self.inner.lock();
        inner.wait_queue.push_back(tid);
    }
}

/// MutexBlocking: interface for syscall layer
pub struct MutexBlocking {
    pub mutex: Mutex,
}

impl MutexBlocking {
    pub fn new() -> Self {
        MutexBlocking {
            mutex: Mutex::new(),
        }
    }
}
