use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Suspended,
    Ready,
    Running,
    Blocked,
}

impl fmt::Display for TaskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskState::Suspended => write!(f, "SUSPENDED"),
            TaskState::Ready => write!(f, "READY"),
            TaskState::Running => write!(f, "RUNNING"),
            TaskState::Blocked => write!(f, "BLOCKED"),
        }
    }
}

/// A segment of a task's job. Each job is a sequence of segments executed in order.
#[derive(Debug, Clone)]
pub enum Segment {
    /// Regular computation (duration in ticks).
    Compute(u32),
    /// Critical section: acquires mutex at segment start, releases at segment end.
    CriticalSection { duration: u32, mutex_id: usize },
}

/// A real-time periodic task.
#[derive(Debug, Clone)]
pub struct Task {
    pub id: usize,
    pub name: String,
    /// Static base priority. Lower value = higher priority. Used for PI reference.
    pub base_priority: u32,
    /// Period of job releases (ticks).
    pub period: u32,
    /// Relative deadline from job release (ticks).
    pub relative_deadline: u32,
    /// Job template: sequence of segments executed each period.
    pub segments: Vec<Segment>,

    // --- Dynamic state ---
    pub state: TaskState,
    /// Absolute deadline of current job instance.
    pub absolute_deadline: u32,
    /// Effective deadline for EDF scheduling. May be reduced by priority inheritance.
    pub effective_deadline: u32,
    /// Next periodic release time.
    pub next_release: u32,
    /// Index of current segment being executed.
    pub current_segment_idx: usize,
    /// Remaining ticks in current segment.
    pub segment_remaining: u32,
    /// Mutex currently held by this task.
    pub held_mutex: Option<usize>,

    // --- Metrics ---
    pub jobs_completed: u32,
    pub deadline_misses: u32,
    pub current_job_release: u32,
    pub response_times: Vec<u32>,
    /// Tick when task became blocked (for measuring inversion duration).
    pub blocked_at: Option<u32>,
}

impl Task {
    pub fn new(
        id: usize,
        name: &str,
        base_priority: u32,
        period: u32,
        relative_deadline: u32,
        segments: Vec<Segment>,
        first_release: u32,
    ) -> Self {
        Task {
            id,
            name: name.to_string(),
            base_priority,
            period,
            relative_deadline,
            segments,
            state: TaskState::Suspended,
            absolute_deadline: u32::MAX,
            effective_deadline: u32::MAX,
            next_release: first_release,
            current_segment_idx: 0,
            segment_remaining: 0,
            held_mutex: None,
            jobs_completed: 0,
            deadline_misses: 0,
            current_job_release: 0,
            response_times: Vec::new(),
            blocked_at: None,
        }
    }

    pub fn wcet(&self) -> u32 {
        self.segments
            .iter()
            .map(|s| match s {
                Segment::Compute(d) => *d,
                Segment::CriticalSection { duration, .. } => *duration,
            })
            .sum()
    }

    /// Release a new job instance at the given time.
    pub fn release_job(&mut self, time: u32) {
        self.state = TaskState::Ready;
        self.absolute_deadline = time + self.relative_deadline;
        self.effective_deadline = self.absolute_deadline;
        self.current_segment_idx = 0;
        self.segment_remaining = match &self.segments[0] {
            Segment::Compute(d) => *d,
            Segment::CriticalSection { duration, .. } => *duration,
        };
        self.held_mutex = None;
        self.current_job_release = time;
        self.blocked_at = None;
        self.next_release += self.period;
    }

    /// Returns true if all segments of the current job have been completed.
    pub fn is_job_complete(&self) -> bool {
        self.current_segment_idx >= self.segments.len()
    }

    /// Advance to the next segment. Returns the next segment reference, or None if job is done.
    pub fn advance_to_next_segment(&mut self) -> Option<&Segment> {
        self.current_segment_idx += 1;
        if self.current_segment_idx < self.segments.len() {
            let segment = &self.segments[self.current_segment_idx];
            self.segment_remaining = match segment {
                Segment::Compute(d) => *d,
                Segment::CriticalSection { duration, .. } => *duration,
            };
            Some(segment)
        } else {
            None
        }
    }

    /// Returns the mutex_id if the current segment is a critical section.
    pub fn current_segment_mutex(&self) -> Option<usize> {
        if self.current_segment_idx < self.segments.len() {
            match &self.segments[self.current_segment_idx] {
                Segment::CriticalSection { mutex_id, .. } => Some(*mutex_id),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Apply jitter to segment durations. Returns a new Task with varied durations.
    pub fn with_jitter(&self, jitter_pct: f64, rng: &mut impl FnMut() -> f64) -> Self {
        let mut task = self.clone();
        task.segments = task
            .segments
            .iter()
            .map(|s| match s {
                Segment::Compute(d) => {
                    let jittered = apply_jitter(*d, jitter_pct, rng);
                    Segment::Compute(jittered)
                }
                Segment::CriticalSection { duration, mutex_id } => {
                    let jittered = apply_jitter(*duration, jitter_pct, rng);
                    Segment::CriticalSection {
                        duration: jittered,
                        mutex_id: *mutex_id,
                    }
                }
            })
            .collect();
        task
    }
}

fn apply_jitter(value: u32, pct: f64, rng: &mut impl FnMut() -> f64) -> u32 {
    let factor = 1.0 + (rng() * 2.0 - 1.0) * pct;
    ((value as f64) * factor).round().max(1.0) as u32
}
