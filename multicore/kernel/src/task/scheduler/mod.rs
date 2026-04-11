pub mod global_rr;
pub mod percpu_rr;

use super::thread::Thread;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Scheduler strategy selection
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SchedulerType {
    GlobalRoundRobin = 0,
    PerCpuWorkStealing = 1,
}

/// Active scheduler type (can be switched at runtime for testing)
static ACTIVE_SCHEDULER: AtomicUsize = AtomicUsize::new(0);

pub fn set_scheduler(stype: SchedulerType) {
    let old_type = get_scheduler_type();
    if old_type == stype {
        return;
    }

    ACTIVE_SCHEDULER.store(stype as usize, Ordering::SeqCst);

    if stype == SchedulerType::PerCpuWorkStealing {
        PERCPU_SCHEDULER.reset_counter();
    }

    // Migrate tasks from the old scheduler to the new one
    let (from, to): (&dyn Scheduler, &dyn Scheduler) = match stype {
        SchedulerType::PerCpuWorkStealing => (&GLOBAL_SCHEDULER, &PERCPU_SCHEDULER),
        SchedulerType::GlobalRoundRobin => (&PERCPU_SCHEDULER, &GLOBAL_SCHEDULER),
    };
    while let Some(task) = from.fetch_task(0) {
        to.add_task(task);
    }

    crate::println!("[kernel] scheduler set to {:?}", stype);
}

pub fn get_scheduler_type() -> SchedulerType {
    match ACTIVE_SCHEDULER.load(Ordering::Relaxed) {
        0 => SchedulerType::GlobalRoundRobin,
        1 => SchedulerType::PerCpuWorkStealing,
        _ => unreachable!(),
    }
}

/// Scheduler trait for multi-core scheduling
pub trait Scheduler: Send + Sync {
    fn add_task(&self, task: Arc<Thread>);
    fn fetch_task(&self, hart_id: usize) -> Option<Arc<Thread>>;
    fn task_count(&self) -> usize;
    #[allow(dead_code)]
    fn name(&self) -> &'static str;
}

static GLOBAL_SCHEDULER: global_rr::GlobalRoundRobin = global_rr::GlobalRoundRobin::new();
static PERCPU_SCHEDULER: percpu_rr::PerCpuWorkStealing = percpu_rr::PerCpuWorkStealing::new();

pub fn active_scheduler() -> &'static dyn Scheduler {
    match get_scheduler_type() {
        SchedulerType::GlobalRoundRobin => &GLOBAL_SCHEDULER,
        SchedulerType::PerCpuWorkStealing => &PERCPU_SCHEDULER,
    }
}

pub fn add_task(task: Arc<Thread>) {
    active_scheduler().add_task(task);
}

pub fn fetch_task(hart_id: usize) -> Option<Arc<Thread>> {
    active_scheduler().fetch_task(hart_id)
}

pub fn task_count() -> usize {
    active_scheduler().task_count()
}
