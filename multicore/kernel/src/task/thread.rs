use super::context::TaskContext;
use super::pid::KernelStack;
use super::process::Process;
use crate::mm::{PhysPageNum, frame_alloc, FrameTracker};
use crate::sync::SpinLock;
use crate::trap::TrapContext;
use alloc::sync::{Arc, Weak};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ThreadStatus {
    Ready,
    Running(usize), // hart_id
    Blocked,
    Exited,
}

pub struct Thread {
    pub tid: usize,
    pub process: Weak<Process>,
    pub kernel_stack: KernelStack,
    pub inner: SpinLock<ThreadInner>,
}

pub struct ThreadInner {
    pub status: ThreadStatus,
    pub task_context: TaskContext,
    pub trap_cx_ppn: PhysPageNum,
    #[allow(dead_code)]
    pub trap_cx_frame: Option<FrameTracker>,
    pub exit_code: Option<i32>,
}

impl ThreadInner {
    pub fn get_trap_cx(&self) -> &'static mut TrapContext {
        self.trap_cx_ppn.get_mut()
    }
}

impl Thread {
    pub fn new(
        process: Weak<Process>,
        tid: usize,
        entry_point: usize,
        user_sp: usize,
        user_arg: usize,
    ) -> Arc<Self> {
        let proc = process.upgrade().unwrap();
        let pid = proc.getpid();

        // Allocate a frame for TrapContext
        let trap_cx_frame = frame_alloc().unwrap();
        let trap_cx_ppn = trap_cx_frame.ppn;

        let kernel_stack = KernelStack::new(pid, tid);
        let kstack_top = kernel_stack.get_top();

        // Initialize trap context
        let trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            crate::mm::kernel_token(),
            kstack_top,
            crate::trap::trap_handler as *const () as usize,
        );
        let mut trap_cx = trap_cx;
        trap_cx.x[10] = user_arg;
        // Set ra to thread-exit trampoline so returning from the entry function
        // calls exit(0) instead of jumping to a garbage address.
        trap_cx.x[1] = crate::config::THREAD_EXIT_TRAMPOLINE;
        *trap_cx_ppn.get_mut::<TrapContext>() = trap_cx;

        let task_context = TaskContext::goto_trap_return(kstack_top);

        Arc::new(Thread {
            tid,
            process,
            kernel_stack,
            inner: SpinLock::new(ThreadInner {
                status: ThreadStatus::Ready,
                task_context,
                trap_cx_ppn,
                trap_cx_frame: Some(trap_cx_frame),
                exit_code: None,
            }),
        })
    }

    /// Create a child thread by forking (for fork syscall)
    pub fn fork_from(
        parent: &Arc<Thread>,
        new_process: Weak<Process>,
    ) -> Arc<Self> {
        let proc = new_process.upgrade().unwrap();
        let pid = proc.getpid();

        let trap_cx_frame = frame_alloc().unwrap();
        let trap_cx_ppn = trap_cx_frame.ppn;

        let kernel_stack = KernelStack::new(pid, 0);
        let kstack_top = kernel_stack.get_top();

        // Copy parent's trap context
        let parent_inner = parent.inner.lock();
        let parent_trap_cx = parent_inner.get_trap_cx();
        let mut child_trap_cx = *parent_trap_cx;
        // Child returns 0 from fork
        child_trap_cx.x[10] = 0;
        child_trap_cx.kernel_sp = kstack_top;
        child_trap_cx.kernel_satp = crate::mm::kernel_token();
        *trap_cx_ppn.get_mut::<TrapContext>() = child_trap_cx;

        let task_context = TaskContext::goto_trap_return(kstack_top);

        Arc::new(Thread {
            tid: 0,
            process: new_process,
            kernel_stack,
            inner: SpinLock::new(ThreadInner {
                status: ThreadStatus::Ready,
                task_context,
                trap_cx_ppn,
                trap_cx_frame: Some(trap_cx_frame),
                exit_code: None,
            }),
        })
    }

    pub fn get_process(&self) -> Arc<Process> {
        self.process.upgrade().unwrap()
    }
}
