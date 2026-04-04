use crate::event::{Event, EventType};
use crate::task::{Task, TaskState};

/// A mutex with optional priority inheritance (PI).
///
/// For EDF scheduling, PI is implemented as *deadline inheritance*:
/// when a high-priority (earlier-deadline) task blocks on this mutex,
/// the owner's effective_deadline is reduced to the blocker's deadline,
/// allowing the owner to preempt medium-priority tasks.
pub struct PiMutex {
    pub id: usize,
    pub owner: Option<usize>,
    pub wait_queue: Vec<usize>,
    pub pi_enabled: bool,
}

impl PiMutex {
    pub fn new(id: usize, pi_enabled: bool) -> Self {
        PiMutex {
            id,
            owner: None,
            wait_queue: Vec::new(),
            pi_enabled,
        }
    }

    /// Try to acquire the mutex for `task_id`.
    /// Returns `true` if acquired immediately, `false` if blocked.
    pub fn try_lock(
        &mut self,
        task_id: usize,
        tasks: &mut [Task],
        events: &mut Vec<Event>,
        current_time: u32,
    ) -> bool {
        match self.owner {
            None => {
                self.owner = Some(task_id);
                tasks[task_id].held_mutex = Some(self.id);
                events.push(Event::new(
                    current_time,
                    task_id,
                    EventType::MutexAcquired(self.id),
                ));
                true
            }
            Some(_) => {
                // Block the requesting task
                tasks[task_id].state = TaskState::Blocked;
                tasks[task_id].blocked_at = Some(current_time);
                self.wait_queue.push(task_id);
                events.push(Event::new(
                    current_time,
                    task_id,
                    EventType::Blocked(self.id),
                ));

                // Apply priority inheritance
                if self.pi_enabled {
                    self.apply_pi(tasks, events, current_time);
                }
                false
            }
        }
    }

    /// Release the mutex held by `task_id`. Wakes up the highest-priority waiter.
    /// Returns the woken task ID, if any.
    pub fn unlock(
        &mut self,
        task_id: usize,
        tasks: &mut [Task],
        events: &mut Vec<Event>,
        current_time: u32,
    ) -> Option<usize> {
        if self.owner != Some(task_id) {
            return None;
        }

        // Restore owner's effective deadline
        let old_eff = tasks[task_id].effective_deadline;
        let original = tasks[task_id].absolute_deadline;
        if old_eff < original {
            tasks[task_id].effective_deadline = original;
            events.push(Event::new(
                current_time,
                task_id,
                EventType::PriorityRestored {
                    original_deadline: original,
                },
            ));
        }

        tasks[task_id].held_mutex = None;
        self.owner = None;
        events.push(Event::new(
            current_time,
            task_id,
            EventType::MutexReleased(self.id),
        ));

        // Wake up the waiter with the earliest effective deadline
        if let Some(&best_tid) = self
            .wait_queue
            .iter()
            .min_by_key(|&&tid| tasks[tid].effective_deadline)
        {
            self.wait_queue.retain(|&tid| tid != best_tid);

            // Transfer ownership
            self.owner = Some(best_tid);
            tasks[best_tid].held_mutex = Some(self.id);
            tasks[best_tid].state = TaskState::Ready;

            events.push(Event::new(
                current_time,
                best_tid,
                EventType::MutexAcquired(self.id),
            ));
            events.push(Event::new(
                current_time,
                best_tid,
                EventType::Unblocked(self.id),
            ));

            Some(best_tid)
        } else {
            None
        }
    }

    /// Priority inheritance: boost the owner's effective deadline
    /// to the minimum of its current value and all waiters' deadlines.
    fn apply_pi(
        &mut self,
        tasks: &mut [Task],
        events: &mut Vec<Event>,
        current_time: u32,
    ) {
        if let Some(owner_id) = self.owner {
            let min_waiter_deadline = self
                .wait_queue
                .iter()
                .map(|&tid| tasks[tid].effective_deadline)
                .min()
                .unwrap_or(u32::MAX);

            if min_waiter_deadline < tasks[owner_id].effective_deadline {
                tasks[owner_id].effective_deadline = min_waiter_deadline;
                events.push(Event::new(
                    current_time,
                    owner_id,
                    EventType::PriorityInherited {
                        new_effective_deadline: min_waiter_deadline,
                    },
                ));
            }
        }
    }
}
