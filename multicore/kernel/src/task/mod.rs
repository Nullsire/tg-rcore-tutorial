pub mod context;
pub mod pid;
pub mod process;
pub mod processor;
pub mod scheduler;
pub mod switch;
pub mod thread;

use alloc::sync::Arc;
use alloc::vec::Vec;
use process::Process;
use thread::{Thread, ThreadStatus};

use crate::sync::SpinLock;

/// Global process list
static PROCESS_LIST: SpinLock<Vec<Option<Arc<Process>>>> = SpinLock::new(Vec::new());

/// Initialize the task system: load initial user programs
pub fn init(app_data: &[&[u8]]) {
    for data in app_data {
        let process = Process::new(data);
        PROCESS_LIST.lock().push(Some(process));
    }
    crate::println!("[kernel] loaded {} initial tasks", app_data.len());
}

/// Add a task to the active scheduler
pub fn add_task(thread: Arc<Thread>) {
    scheduler::add_task(thread);
}

/// Suspend current thread and run the next one
pub fn suspend_current_and_run_next() {
    let task = processor::take_current_task().unwrap();
    let mut inner = task.inner.lock();
    inner.status = ThreadStatus::Ready;
    let task_cx_ptr = &mut inner.task_context as *mut context::TaskContext;
    drop(inner);

    // Defer enqueue: store the task so it gets added to the scheduler
    // AFTER __switch saves its context. This prevents another hart from
    // picking up the task with a stale (not yet saved) TaskContext.
    unsafe {
        processor::current_processor().pending_enqueue = Some(task);
    }
    processor::schedule(task_cx_ptr);
}

/// Exit current thread and run the next one
pub fn exit_current_and_run_next(exit_code: i32) {
    let task = processor::take_current_task().unwrap();
    let mut inner = task.inner.lock();
    inner.status = ThreadStatus::Exited;
    inner.exit_code = Some(exit_code);
    let task_cx_ptr = &mut inner.task_context as *mut context::TaskContext;
    drop(inner);

    // Check if all threads of the process are exited
    let process = task.get_process();
    let mut proc_inner = process.inner.lock();

    // Mark this thread as done
    if let Some(slot) = proc_inner.threads.get_mut(task.tid) {
        *slot = None;
    }

    // Check if all threads are done
    let all_exited = proc_inner.threads.iter().all(|t| t.is_none());
    if all_exited {
        proc_inner.is_zombie = true;
        proc_inner.exit_code = exit_code;

        // Move children to init process
        // (simplified: just clear children)
        proc_inner.children.clear();
    }
    drop(proc_inner);

    // Store task for later cleanup (will be dropped in idle loop)
    unsafe {
        processor::current_processor().dead_task = Some(task);
    }

    processor::schedule(task_cx_ptr);
}

/// Block current thread
pub fn block_current_task() -> Arc<Thread> {
    let task = processor::take_current_task().unwrap();
    let mut inner = task.inner.lock();
    inner.status = ThreadStatus::Blocked;
    let task_cx_ptr = &mut inner.task_context as *mut context::TaskContext;
    drop(inner);
    let task_clone = task.clone();
    processor::schedule(task_cx_ptr);
    task_clone
}

/// Unblock a thread (add it back to scheduler)
#[allow(dead_code)]
pub fn unblock_task(task: Arc<Thread>) {
    let mut inner = task.inner.lock();
    inner.status = ThreadStatus::Ready;
    drop(inner);
    scheduler::add_task(task);
}

/// Add process to global list
pub fn add_process(process: Arc<Process>) {
    let pid = process.getpid();
    let mut list = PROCESS_LIST.lock();
    if pid >= list.len() {
        list.resize(pid + 1, None);
    }
    list[pid] = Some(process);
}

pub fn find_process(pid: usize) -> Option<Arc<Process>> {
    let list = PROCESS_LIST.lock();
    if pid >= list.len() {
        return None;
    }
    list[pid].as_ref().cloned()
}

/// Remove a zombie process from the global list so its resources are freed.
pub fn remove_process(pid: usize) {
    let mut list = PROCESS_LIST.lock();
    if pid < list.len() {
        list[pid] = None;
    }
}

pub fn send_signal(pid: usize, signal: usize) -> isize {
    if signal == 0 || signal >= (usize::BITS as usize) {
        return -1;
    }
    let Some(process) = find_process(pid) else {
        return -1;
    };
    let mut inner = process.inner.lock();
    if inner.is_zombie {
        return -1;
    }
    inner.signal_pending |= 1usize << signal;
    0
}

pub fn take_current_deliverable_signal() -> Option<usize> {
    let task = processor::current_task()?;
    let process = task.get_process();
    let mut inner = process.inner.lock();
    let deliverable = inner.signal_pending & !inner.signal_mask;
    if deliverable == 0 {
        return None;
    }
    let sig = deliverable.trailing_zeros() as usize;
    inner.signal_pending &= !(1usize << sig);
    Some(sig)
}
