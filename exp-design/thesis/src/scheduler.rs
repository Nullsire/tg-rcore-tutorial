use crate::task::{Task, TaskState};

/// Earliest Deadline First (EDF) scheduler.
/// Picks the ready task with the smallest effective_deadline.
/// Ties broken by base_priority (lower = higher priority).
pub struct EdfScheduler;

impl EdfScheduler {
    pub fn pick_next(tasks: &[Task]) -> Option<usize> {
        tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.state == TaskState::Ready)
            .min_by(|(_, a), (_, b)| {
                a.effective_deadline
                    .cmp(&b.effective_deadline)
                    .then(a.base_priority.cmp(&b.base_priority))
            })
            .map(|(i, _)| i)
    }
}
