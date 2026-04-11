use super::pid::{pid_alloc, PidHandle};
use super::thread::Thread;
use crate::mm::MemorySet;
use crate::sync::SpinLock;
use crate::trap::TrapContext;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;

pub struct Process {
    pub pid: PidHandle,
    pub inner: SpinLock<ProcessInner>,
}

pub struct ProcessInner {
    pub is_zombie: bool,
    pub memory_set: MemorySet,
    #[allow(dead_code)]
    pub parent: Option<Weak<Process>>,
    pub children: Vec<Arc<Process>>,
    pub exit_code: i32,
    pub threads: Vec<Option<Arc<Thread>>>,
    pub fd_table: Vec<Option<Arc<dyn crate::fs::File + Send + Sync>>>,
    pub signal_mask: usize,
    pub signal_pending: usize,
    pub next_user_stack_top: usize,
}

impl Process {
    pub fn new(elf_data: &[u8]) -> Arc<Self> {
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf_data);
        let pid_handle = pid_alloc();
        let _pid = pid_handle.0;

        // Next thread stack starts above the initial stack, with a guard page gap
        let next_stack = user_sp + crate::config::PAGE_SIZE + crate::config::USER_STACK_SIZE;

        let process = Arc::new(Process {
            pid: pid_handle,
            inner: SpinLock::new(ProcessInner {
                is_zombie: false,
                memory_set,
                parent: None,
                children: Vec::new(),
                exit_code: 0,
                threads: Vec::new(),
                fd_table: vec![
                    // 0: stdin
                    Some(Arc::new(crate::fs::Stdin)),
                    // 1: stdout
                    Some(Arc::new(crate::fs::Stdout)),
                    // 2: stderr
                    Some(Arc::new(crate::fs::Stdout)),
                ],
                signal_mask: 0,
                signal_pending: 0,
                next_user_stack_top: next_stack,
            }),
        });

        // Create main thread
        let thread = Thread::new(
            Arc::downgrade(&process),
            0, // tid = 0 for main thread
            entry_point,
            user_sp,
            0,
        );

        process.inner.lock().threads.push(Some(thread.clone()));
        // Add to scheduler
        super::add_task(thread);

        process
    }

    pub fn getpid(&self) -> usize {
        self.pid.0
    }

    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        let mut parent_inner = self.inner.lock();
        let memory_set = MemorySet::from_existing(&mut parent_inner.memory_set);
        let pid_handle = pid_alloc();

        // Copy fd table
        let mut new_fd_table: Vec<Option<Arc<dyn crate::fs::File + Send + Sync>>> = Vec::new();
        for fd in parent_inner.fd_table.iter() {
            new_fd_table.push(fd.clone());
        }

        let child = Arc::new(Process {
            pid: pid_handle,
            inner: SpinLock::new(ProcessInner {
                is_zombie: false,
                memory_set,
                parent: Some(Arc::downgrade(self)),
                children: Vec::new(),
                exit_code: 0,
                threads: Vec::new(),
                fd_table: new_fd_table,
                signal_mask: parent_inner.signal_mask,
                signal_pending: 0,
                next_user_stack_top: parent_inner.next_user_stack_top,
            }),
        });

        // Create child's main thread (copy of parent's main thread)
        let parent_thread = parent_inner.threads[0].as_ref().unwrap();
        let child_thread = Thread::fork_from(
            parent_thread,
            Arc::downgrade(&child),
        );

        child.inner.lock().threads.push(Some(child_thread.clone()));
        parent_inner.children.push(child.clone());

        // Add child thread to scheduler
        super::add_task(child_thread);

        child
    }

    pub fn exec(&self, elf_data: &[u8]) {
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf_data);
        let mut inner = self.inner.lock();
        inner.memory_set = memory_set;
        inner.signal_pending = 0;

        // Reset the main thread's trap context
        let thread = inner.threads[0].as_ref().unwrap();
        let thread_inner = thread.inner.lock();
        let trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            crate::mm::kernel_token(),
            thread.kernel_stack.get_top(),
            crate::trap::trap_handler as *const () as usize,
        );
        *thread_inner.get_trap_cx() = trap_cx;
    }
}
