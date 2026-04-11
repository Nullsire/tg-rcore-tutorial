use crate::config::{KERNEL_STACK_SIZE, PAGE_SIZE};
use crate::sync::SpinLock;
use alloc::alloc::{alloc_zeroed, dealloc, Layout};
use alloc::vec::Vec;

struct PidAllocator {
    current: usize,
    recycled: Vec<usize>,
}

impl PidAllocator {
    fn alloc(&mut self) -> PidHandle {
        if let Some(pid) = self.recycled.pop() {
            PidHandle(pid)
        } else {
            self.current += 1;
            PidHandle(self.current - 1)
        }
    }

    fn dealloc(&mut self, pid: usize) {
        assert!(pid < self.current);
        assert!(!self.recycled.contains(&pid));
        self.recycled.push(pid);
    }
}

static PID_ALLOCATOR: SpinLock<PidAllocator> = SpinLock::new(PidAllocator {
    current: 0,
    recycled: Vec::new(),
});

pub struct PidHandle(pub usize);

impl Drop for PidHandle {
    fn drop(&mut self) {
        PID_ALLOCATOR.lock().dealloc(self.0);
    }
}

pub fn pid_alloc() -> PidHandle {
    PID_ALLOCATOR.lock().alloc()
}

/// Kernel stack backed by physical frames (identity-mapped).
/// Allocates one contiguous, page-aligned heap region.
pub struct KernelStack {
    base: usize,
    layout: Layout,
}

impl KernelStack {
    pub fn new(_pid: usize, _tid: usize) -> Self {
        let layout = Layout::from_size_align(KERNEL_STACK_SIZE, PAGE_SIZE)
            .expect("invalid kernel stack layout");
        let ptr = unsafe { alloc_zeroed(layout) };
        let base = ptr as usize;
        assert!(base != 0, "failed to allocate kernel stack");
        KernelStack { base, layout }
    }

    /// Get the top of the kernel stack (highest address + 1).
    pub fn get_top(&self) -> usize {
        self.base + self.layout.size()
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        unsafe { dealloc(self.base as *mut u8, self.layout) };
    }
}
