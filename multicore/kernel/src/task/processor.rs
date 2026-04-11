use super::context::TaskContext;
use super::switch::__switch;
use super::thread::{Thread, ThreadStatus};
use crate::config::MAX_HARTS;
use crate::trap::TrapContext;
use alloc::sync::Arc;
use core::arch::asm;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Per-hart processor state
pub struct Processor {
    pub hart_id: usize,
    pub current: Option<Arc<Thread>>,
    pub idle_task_cx: TaskContext,
    pub need_reschedule: bool,
    pub dead_task: Option<Arc<Thread>>,
    /// Task waiting to be enqueued after its context is saved by __switch.
    /// This avoids the race where another hart picks up the task before
    /// the suspending hart has saved its TaskContext.
    pub pending_enqueue: Option<Arc<Thread>>,
}

impl Processor {
    pub const fn new(hart_id: usize) -> Self {
        Processor {
            hart_id,
            current: None,
            idle_task_cx: TaskContext {
                ra: 0,
                sp: 0,
                s: [0; 12],
            },
            need_reschedule: false,
            dead_task: None,
            pending_enqueue: None,
        }
    }
}

/// Array of per-hart processors
static mut PROCESSORS: [Processor; MAX_HARTS] = [
    Processor::new(0),
    Processor::new(1),
    Processor::new(2),
    Processor::new(3),
];

static ACTIVE_HARTS: AtomicUsize = AtomicUsize::new(MAX_HARTS);
static ACTIVE_HART_BASE: AtomicUsize = AtomicUsize::new(0);

pub fn set_active_harts(count: usize) -> bool {
    if (1..=MAX_HARTS).contains(&count) {
        if count == 1 {
            // Keep the caller's hart active in single-hart mode, regardless of its numeric id.
            ACTIVE_HART_BASE.store(hart_id(), Ordering::SeqCst);
        } else {
            ACTIVE_HART_BASE.store(0, Ordering::SeqCst);
        }
        ACTIVE_HARTS.store(count, Ordering::SeqCst);
        true
    } else {
        false
    }
}

pub fn active_harts() -> usize {
    ACTIVE_HARTS.load(Ordering::Relaxed)
}

/// Map physical hart id to a logical scheduler index.
/// Returns None when this hart is inactive under the current active-hart policy.
pub fn logical_hart_index(physical_hart: usize) -> Option<usize> {
    let active = active_harts();
    if active == 1 {
        let base = ACTIVE_HART_BASE.load(Ordering::Relaxed);
        if physical_hart == base {
            Some(0)
        } else {
            None
        }
    } else if physical_hart < active {
        Some(physical_hart)
    } else {
        None
    }
}

/// Initialize the processor for the current hart
pub fn init_processor(hart_id: usize) {
    unsafe {
        PROCESSORS[hart_id].hart_id = hart_id;
        // Set tp register to hart_id for fast access
        asm!("mv tp, {}", in(reg) hart_id);
    }
}

/// Get current hart ID from tp register
#[inline]
pub fn hart_id() -> usize {
    let id: usize;
    unsafe {
        asm!("mv {}, tp", out(reg) id);
    }
    id
}

/// Get current processor (mutable, unsafe because no locking)
pub unsafe fn current_processor() -> &'static mut Processor {
    &mut PROCESSORS[hart_id()]
}

/// Take current thread from processor
pub fn take_current_task() -> Option<Arc<Thread>> {
    unsafe { current_processor().current.take() }
}

/// Get reference to current thread
pub fn current_task() -> Option<Arc<Thread>> {
    unsafe { current_processor().current.clone() }
}

/// Get current trap context
pub fn current_trap_cx() -> &'static mut TrapContext {
    let thread = current_task().unwrap();
    let inner = thread.inner.lock();
    inner.trap_cx_ppn.get_mut()
}

/// Get current user's virtual address for trap context
pub fn current_trap_cx_user_va() -> usize {
    let thread = current_task().unwrap();
    let inner = thread.inner.lock();
    let ppn = inner.trap_cx_ppn;
    let pa: crate::mm::PhysAddr = ppn.into();
    pa.0
}

/// Get current user's satp token
pub fn current_user_token() -> usize {
    let thread = current_task().unwrap();
    let process = thread.get_process();
    let inner = process.inner.lock();
    inner.memory_set.token()
}

/// Run tasks on this hart (idle loop + scheduling)
pub fn run_tasks() -> ! {
    let hid = hart_id();
    loop {
        // Clean up any previously exited task
        unsafe {
            if let Some(dead) = current_processor().dead_task.take() {
                drop(dead);
            }
        }

        if logical_hart_index(hid).is_none() {
            // Inactive hart: just wait for interrupts
            unsafe {
                asm!("csrs sstatus, {}", in(reg) 1usize << 1);
                asm!("wfi");
                asm!("csrc sstatus, {}", in(reg) 1usize << 1);
            }
            continue;
        }

        // Try to fetch a task
        if let Some(task) = super::scheduler::fetch_task(hid) {
            let idle_task_cx_ptr;
            let next_task_cx_ptr;

            {
                let mut task_inner = task.inner.lock();
                task_inner.status = ThreadStatus::Running(hid);
                next_task_cx_ptr = &task_inner.task_context as *const TaskContext;
            }

            unsafe {
                let processor = current_processor();
                idle_task_cx_ptr = &mut processor.idle_task_cx as *mut TaskContext;
                processor.current = Some(task);
            }

            unsafe {
                __switch(idle_task_cx_ptr, next_task_cx_ptr);
            }

            // Context of previous task is now saved. Safe to enqueue it.
            unsafe {
                if let Some(pending) = current_processor().pending_enqueue.take() {
                    super::scheduler::add_task(pending);
                }
            }

            // Returned from task, back in idle loop
        } else {
            // No tasks available, enable interrupts and wait
            unsafe {
                asm!("csrs sstatus, {}", in(reg) 1usize << 1); // enable SIE
                asm!("wfi"); // wait for interrupt (timer or IPI)
                asm!("csrc sstatus, {}", in(reg) 1usize << 1); // disable SIE
            }
        }
    }
}

/// Schedule: switch from current task back to idle loop
pub fn schedule(switched_task_cx_ptr: *mut TaskContext) {
    let idle_task_cx_ptr;
    unsafe {
        idle_task_cx_ptr = &current_processor().idle_task_cx as *const TaskContext;
    }
    unsafe {
        __switch(switched_task_cx_ptr, idle_task_cx_ptr);
    }
}
