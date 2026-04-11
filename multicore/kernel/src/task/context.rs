/// Task context saved during context switch.
/// Only callee-saved registers need to be saved (ra, sp, s0-s11).
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct TaskContext {
    pub ra: usize,
    pub sp: usize,
    pub s: [usize; 12],
}

impl TaskContext {
    #[allow(dead_code)]
    pub fn zero_init() -> Self {
        Self {
            ra: 0,
            sp: 0,
            s: [0; 12],
        }
    }

    /// Create a task context that will jump to trap_return_wrapper when scheduled.
    /// When __switch returns, ra takes us to the wrapper that properly
    /// sets up parameters for user_trap_return assembly.
    pub fn goto_trap_return(kstack_ptr: usize) -> Self {
        Self {
            ra: crate::trap::trap_return_wrapper as *const () as usize,
            sp: kstack_ptr,
            s: [0; 12],
        }
    }
}
