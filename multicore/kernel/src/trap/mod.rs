pub mod context;
pub mod interrupt;

use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicBool, Ordering};

global_asm!(include_str!("trap.S"));

pub use context::TrapContext;

extern "C" {
    pub fn user_trap_vector();
    pub fn user_trap_return();
    pub fn kernel_trap_vector();
}

const CAUSE_INTERRUPT_BIT: usize = 1 << 63;
const CAUSE_SUPERVISOR_TIMER: usize = CAUSE_INTERRUPT_BIT | 5;
const CAUSE_SUPERVISOR_EXTERNAL: usize = CAUSE_INTERRUPT_BIT | 9;
const CAUSE_SUPERVISOR_SOFT: usize = CAUSE_INTERRUPT_BIT | 1;
const CAUSE_USER_ECALL: usize = 8;
const CAUSE_STORE_PAGE_FAULT: usize = 15;
const CAUSE_LOAD_PAGE_FAULT: usize = 13;
const CAUSE_INSTRUCTION_PAGE_FAULT: usize = 12;
const CAUSE_ILLEGAL_INSTRUCTION: usize = 2;

static KERNEL_IRQ_ENABLED: AtomicBool = AtomicBool::new(true);

/// Initialize trap handling for this hart
pub fn init(hart_id: usize) {
    unsafe {
        asm!("csrw stvec, {}", in(reg) kernel_trap_vector as *const () as usize);
        // Enable: SSIE(1) | STIE(5) | SEIE(9)
        asm!("csrw sie, {}", in(reg) (1 << 1) | (1 << 5) | (1 << 9));
    }
    interrupt::plic_init(hart_id);
    crate::println!("[kernel] hart {} trap handler initialized", hart_id);
}

#[allow(dead_code)]
pub fn enable_interrupt() {
    unsafe {
        asm!("csrs sstatus, {}", in(reg) 1usize << 1);
    }
}

pub fn set_kernel_irq_enabled(enabled: bool) {
    KERNEL_IRQ_ENABLED.store(enabled, Ordering::SeqCst);
}

pub fn is_kernel_irq_enabled() -> bool {
    KERNEL_IRQ_ENABLED.load(Ordering::Relaxed)
}

pub fn disable_interrupt() {
    unsafe {
        asm!("csrc sstatus, {}", in(reg) 1usize << 1);
    }
}

/// User trap handler - called from user_trap_vector assembly
#[no_mangle]
pub fn trap_handler() -> ! {
    // Set stvec to kernel_trap_vector for kernel interrupts
    unsafe {
        asm!("csrw stvec, {}", in(reg) kernel_trap_vector as *const () as usize);
    }

    let scause: usize;
    let stval: usize;
    let sepc: usize;
    unsafe {
        asm!("csrr {}, scause", out(reg) scause);
        asm!("csrr {}, stval", out(reg) stval);
        asm!("csrr {}, sepc", out(reg) sepc);
    }

    match scause {
        CAUSE_USER_ECALL => {
            let cx = crate::task::processor::current_trap_cx();
            cx.sepc += 4;
            let result = crate::syscall::syscall(
                cx.x[17],
                [cx.x[10], cx.x[11], cx.x[12]],
            );
            let cx = crate::task::processor::current_trap_cx();
            cx.x[10] = result as usize;
        }
        CAUSE_SUPERVISOR_TIMER => {
            interrupt::timer_handler();
        }
        CAUSE_SUPERVISOR_EXTERNAL => {
            interrupt::handle_external_irq();
        }
        CAUSE_SUPERVISOR_SOFT => {
            // IPI received while in user mode — just clear pending bit
            unsafe {
                asm!("csrc sip, {}", in(reg) 1usize << 1);
            }
        }
        CAUSE_STORE_PAGE_FAULT => {
            // Try COW resolution first
            let vpn = crate::mm::VirtPageNum(stval / crate::config::PAGE_SIZE);
            let task = crate::task::processor::current_task().unwrap();
            let process = task.get_process();
            let mut inner = process.inner.lock();
            if inner.memory_set.handle_cow_fault(vpn) {
                // COW resolved, retry the instruction
                drop(inner);
            } else {
                crate::println!(
                    "[kernel] store page fault: stval={:#x}, sepc={:#x}",
                    stval, sepc
                );
                drop(inner);
                crate::task::exit_current_and_run_next(-2);
            }
        }
        CAUSE_LOAD_PAGE_FAULT | CAUSE_INSTRUCTION_PAGE_FAULT => {
            crate::println!(
                "[kernel] page fault: scause={:#x}, stval={:#x}, sepc={:#x}",
                scause, stval, sepc
            );
            crate::task::exit_current_and_run_next(-2);
        }
        CAUSE_ILLEGAL_INSTRUCTION => {
            crate::println!(
                "[kernel] illegal instruction at {:#x}, stval={:#x}",
                sepc, stval
            );
            crate::task::exit_current_and_run_next(-3);
        }
        _ => {
            panic!(
                "unsupported trap: scause={:#x}, stval={:#x}, sepc={:#x}",
                scause, stval, sepc
            );
        }
    }

    if let Some(sig) = crate::task::take_current_deliverable_signal() {
        // Minimal signal semantics: deliver pending unmasked signal and apply
        // default action (terminate process).
        crate::task::exit_current_and_run_next(-(128 + sig as i32));
    }

    // Check reschedule before returning to user
    if unsafe { crate::task::processor::current_processor().need_reschedule } {
        unsafe {
            crate::task::processor::current_processor().need_reschedule = false;
        }
        crate::task::suspend_current_and_run_next();
    }

    trap_return_to_user();
}

/// Kernel trap handler - called from kernel_trap_vector when interrupted in S-mode
#[no_mangle]
pub fn kernel_trap_handler(_stval: usize, scause: usize, _sepc: usize) {
    match scause {
        CAUSE_SUPERVISOR_TIMER => {
            interrupt::kernel_timer_handler();
        }
        CAUSE_SUPERVISOR_EXTERNAL => {
            interrupt::kernel_external_handler();
        }
        CAUSE_SUPERVISOR_SOFT => {
            // IPI
            unsafe {
                asm!("csrc sip, {}", in(reg) 1usize << 1);
            }
        }
        CAUSE_STORE_PAGE_FAULT | CAUSE_LOAD_PAGE_FAULT | CAUSE_INSTRUCTION_PAGE_FAULT => {
            let sp_val: usize;
            let ra_val: usize;
            unsafe {
                asm!("mv {}, sp", out(reg) sp_val);
                asm!("mv {}, ra", out(reg) ra_val);
            }
            crate::println!(
                "[kernel] {} page fault in S-mode: sepc={:#x}, stval={:#x}, sp={:#x}, ra={:#x}, hart={}",
                match scause {
                    CAUSE_STORE_PAGE_FAULT => "Store",
                    CAUSE_LOAD_PAGE_FAULT => "Load",
                    _ => "Instruction",
                },
                _sepc, _stval, sp_val, ra_val,
                crate::task::processor::hart_id()
            );
            // Dump current task info if available
            if let Some(task) = crate::task::processor::current_task() {
                let inner = task.inner.lock();
                crate::println!(
                    "[kernel] current task: tid={}, status={:?}",
                    task.tid, inner.status
                );
            } else {
                crate::println!("[kernel] no current task (idle?)");
            }
            panic!("kernel page fault");
        }
        _ => {
            panic!(
                "kernel trap: scause={:#x}, stval={:#x}, sepc={:#x}",
                scause, _stval, _sepc
            );
        }
    }
}

/// Called when a newly scheduled task runs for the first time.
/// Sets up parameters and jumps to user_trap_return assembly.
#[no_mangle]
pub fn trap_return_wrapper() -> ! {
    trap_return_to_user();
}

#[no_mangle]
#[inline(never)]
fn trap_return_to_user() -> ! {
    disable_interrupt();
    let trap_cx_ptr = crate::task::processor::current_trap_cx_user_va();
    let user_satp = crate::task::processor::current_user_token();
    let stvec_val = user_trap_vector as *const () as usize;
    let fn_ptr = user_trap_return as *const () as usize;
    unsafe {
        asm!(
            "csrw stvec, {stvec}",
            "fence.i",
            "jr {fn_ptr}",
            stvec = in(reg) stvec_val,
            fn_ptr = in(reg) fn_ptr,
            in("a0") trap_cx_ptr,
            in("a1") user_satp,
            options(noreturn),
        );
    }
}
