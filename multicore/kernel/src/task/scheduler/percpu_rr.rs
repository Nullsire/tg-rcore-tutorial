use super::Scheduler;
use crate::config::MAX_HARTS;
use crate::sync::SpinLock;
use crate::task::thread::Thread;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Per-CPU Round-Robin Scheduler with Work Stealing
/// Each hart has its own ready queue. When a hart's queue is empty,
/// it steals from other harts.
pub struct PerCpuWorkStealing {
    queues: [SpinLock<VecDeque<Arc<Thread>>>; MAX_HARTS],
    total_count: AtomicUsize,
    next_hart: AtomicUsize, // for load balancing on add
}

impl PerCpuWorkStealing {
    pub const fn new() -> Self {
        PerCpuWorkStealing {
            queues: [
                SpinLock::new(VecDeque::new()),
                SpinLock::new(VecDeque::new()),
                SpinLock::new(VecDeque::new()),
                SpinLock::new(VecDeque::new()),
            ],
            total_count: AtomicUsize::new(0),
            next_hart: AtomicUsize::new(0),
        }
    }
}

impl Scheduler for PerCpuWorkStealing {
    fn add_task(&self, task: Arc<Thread>) {
        let active = crate::task::processor::active_harts().max(1);
        // Use PID for deterministic queue placement. Consecutive PIDs
        // (e.g. from fork loops) spread evenly across harts, and
        // re-added tasks (after yield) return to the same queue.
        let target = task.get_process().getpid() % active;
        let was_empty = {
            let mut q = self.queues[target].lock();
            let empty = q.is_empty();
            q.push_back(task);
            empty
        };
        self.total_count.fetch_add(1, Ordering::Relaxed);

        if was_empty {
            // Rely on timer interrupts (10ms) to wake idle harts.
        }
    }

    fn fetch_task(&self, hart_id: usize) -> Option<Arc<Thread>> {
        let active = crate::task::processor::active_harts().max(1);
        let local_idx = match crate::task::processor::logical_hart_index(hart_id) {
            Some(idx) => idx,
            None => return None,
        };
        if local_idx >= active {
            return None;
        }

        // Try local queue first
        if let Some(task) = self.queues[local_idx].lock().pop_front() {
            self.total_count.fetch_sub(1, Ordering::Relaxed);
            return Some(task);
        }

        // Work stealing: try other harts' queues
        // Only steal when victim has > 1 task — don't take the last one.
        for i in 1..active {
            let victim = (local_idx + i) % active;
            let mut victim_queue = self.queues[victim].lock();
            if victim_queue.len() > 2 {
                // Steal half of the victim's tasks (at least 1)
                let steal_count = victim_queue.len() / 2;
                let steal_count = steal_count.max(1);
                let mut stolen = Vec::new();
                for _ in 0..steal_count {
                    if let Some(task) = victim_queue.pop_back() {
                        stolen.push(task);
                    }
                }
                drop(victim_queue);

                if !stolen.is_empty() {
                    let first = stolen.remove(0);
                    self.total_count.fetch_sub(1, Ordering::Relaxed);

                    // Put remaining stolen tasks in local queue
                    if !stolen.is_empty() {
                        let mut local = self.queues[local_idx].lock();
                        for t in stolen {
                            local.push_back(t);
                        }
                    }
                    return Some(first);
                }
            }
        }

        None
    }

    fn task_count(&self) -> usize {
        self.total_count.load(Ordering::Relaxed)
    }

    fn name(&self) -> &'static str {
        "Per-CPU Work-Stealing"
    }
}

impl PerCpuWorkStealing {
    pub fn reset_counter(&self) {
        self.next_hart.store(0, Ordering::SeqCst);
    }
}

use alloc::vec::Vec;
