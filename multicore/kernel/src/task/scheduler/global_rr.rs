use super::Scheduler;
use crate::sync::SpinLock;
use crate::task::thread::Thread;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Global Round-Robin Scheduler
/// All harts share a single ready queue protected by one spinlock.
pub struct GlobalRoundRobin {
    queue: SpinLock<VecDeque<Arc<Thread>>>,
    count: AtomicUsize,
}

impl GlobalRoundRobin {
    pub const fn new() -> Self {
        GlobalRoundRobin {
            queue: SpinLock::new(VecDeque::new()),
            count: AtomicUsize::new(0),
        }
    }
}

impl Scheduler for GlobalRoundRobin {
    fn add_task(&self, task: Arc<Thread>) {
        let was_empty = {
            let mut q = self.queue.lock();
            let empty = q.is_empty();
            q.push_back(task);
            empty
        };
        self.count.fetch_add(1, Ordering::Relaxed);

        if was_empty {
            // Rely on timer interrupts (10ms) to wake idle harts.
        }
    }

    fn fetch_task(&self, _hart_id: usize) -> Option<Arc<Thread>> {
        let task = self.queue.lock().pop_front();
        if task.is_some() {
            self.count.fetch_sub(1, Ordering::Relaxed);
        }
        task
    }

    fn task_count(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    fn name(&self) -> &'static str {
        "Global Round-Robin"
    }
}
