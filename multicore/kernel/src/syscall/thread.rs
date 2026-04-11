use crate::task;
use crate::task::processor::current_task;
use crate::task::thread::Thread;
use alloc::sync::Arc;

pub fn sys_thread_create(entry: usize, user_sp: usize, arg: usize) -> isize {
    let task = current_task().unwrap();
    let process = task.get_process();

    let effective_user_sp = if user_sp == 0 {
        // Allocate an independent user stack for this thread
        let mut proc_inner = process.inner.lock();
        let new_stack_top = proc_inner.next_user_stack_top;
        proc_inner.memory_set.alloc_user_stack(new_stack_top);
        // Reserve space for next thread: guard page + stack
        proc_inner.next_user_stack_top =
            new_stack_top + crate::config::PAGE_SIZE + crate::config::USER_STACK_SIZE;
        drop(proc_inner);
        new_stack_top
    } else {
        user_sp
    };

    let tid = {
        let proc_inner = process.inner.lock();
        proc_inner.threads.len()
    };

    let new_thread = Thread::new(
        Arc::downgrade(&process),
        tid,
        entry,
        effective_user_sp,
        arg,
    );

    {
        let mut proc_inner = process.inner.lock();
        proc_inner.threads.push(Some(new_thread.clone()));
    }

    task::add_task(new_thread);
    tid as isize
}

pub fn sys_gettid() -> isize {
    let task = current_task().unwrap();
    task.tid as isize
}

pub fn sys_waittid(tid: usize) -> isize {
    let task = current_task().unwrap();
    let process = task.get_process();

    loop {
        let proc_inner = process.inner.lock();
        if tid >= proc_inner.threads.len() {
            return -1;
        }
        if proc_inner.threads[tid].is_none() {
            // Thread already exited and cleaned up
            return 0;
        }
        let thread = proc_inner.threads[tid].as_ref().unwrap();
        let thread_inner = thread.inner.lock();
        if let Some(exit_code) = thread_inner.exit_code {
            drop(thread_inner);
            drop(proc_inner);
            // Clean up the thread slot
            let mut proc_inner = process.inner.lock();
            proc_inner.threads[tid] = None;
            return exit_code as isize;
        }
        drop(thread_inner);
        drop(proc_inner);

        task::suspend_current_and_run_next();
    }
}
