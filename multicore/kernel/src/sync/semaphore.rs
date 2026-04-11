use super::SpinLock;
use alloc::collections::VecDeque;

pub struct Semaphore {
    inner: SpinLock<SemaphoreInner>,
}

struct SemaphoreInner {
    count: isize,
    wait_queue: VecDeque<usize>,
}

impl Semaphore {
    pub fn new(count: usize) -> Self {
        Semaphore {
            inner: SpinLock::new(SemaphoreInner {
                count: count as isize,
                wait_queue: VecDeque::new(),
            }),
        }
    }

    /// Returns true if acquired, false if must block
    pub fn down(&self) -> bool {
        let mut inner = self.inner.lock();
        inner.count -= 1;
        inner.count >= 0
    }

    #[allow(dead_code)]
    pub fn add_waiter(&self, tid: usize) {
        let mut inner = self.inner.lock();
        inner.wait_queue.push_back(tid);
    }

    /// Returns a thread to wake, if any
    pub fn up(&self) -> Option<usize> {
        let mut inner = self.inner.lock();
        inner.count += 1;
        inner.wait_queue.pop_front()
    }
}
