/// Event types recorded during simulation for analysis.
#[derive(Debug, Clone)]
pub struct Event {
    pub time: u32,
    pub task_id: usize,
    pub event_type: EventType,
}

#[derive(Debug, Clone)]
pub enum EventType {
    Released,
    Scheduled,
    Preempted,
    Blocked(usize),        // mutex_id
    Unblocked(usize),      // mutex_id
    MutexAcquired(usize),  // mutex_id
    MutexReleased(usize),  // mutex_id
    JobCompleted { response_time: u32 },
    DeadlineMissed { deadline: u32, completed_at: u32 },
    PriorityInherited { new_effective_deadline: u32 },
    PriorityRestored { original_deadline: u32 },
}

impl Event {
    pub fn new(time: u32, task_id: usize, event_type: EventType) -> Self {
        Event { time, task_id, event_type }
    }
}
