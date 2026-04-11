use core::arch::asm;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TrapContext {
    /// General purpose registers x0-x31
    pub x: [usize; 32],
    /// Supervisor Status Register
    pub sstatus: usize,
    /// Supervisor Exception Program Counter
    pub sepc: usize,
    /// Kernel satp (for trap handler to switch to kernel page table)
    pub kernel_satp: usize,
    /// Kernel stack pointer for this thread
    pub kernel_sp: usize,
    /// Address of trap handler function
    pub trap_handler: usize,
}

impl TrapContext {
    pub fn set_sp(&mut self, sp: usize) {
        self.x[2] = sp;
    }

    pub fn app_init_context(
        entry: usize,
        sp: usize,
        kernel_satp: usize,
        kernel_sp: usize,
        trap_handler: usize,
    ) -> Self {
        let mut sstatus: usize;
        unsafe {
            asm!("csrr {}, sstatus", out(reg) sstatus);
        }
        // Set SPP to User mode (clear bit 8)
        sstatus &= !(1 << 8);
        // Enable SPIE so interrupts are enabled after sret
        sstatus |= 1 << 5;
        let mut cx = TrapContext {
            x: [0; 32],
            sstatus,
            sepc: entry,
            kernel_satp,
            kernel_sp,
            trap_handler,
        };
        cx.set_sp(sp);
        cx
    }
}

/// Kernel trap frame: saved on kernel stack when interrupted in S-mode
#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KernelTrapContext {
    pub ra: usize,
    pub sp: usize,
    pub s0: usize,
    pub s1: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub sstatus: usize,
    pub sepc: usize,
}
