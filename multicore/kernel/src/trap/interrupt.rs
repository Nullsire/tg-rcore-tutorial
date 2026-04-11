use crate::config::*;
use crate::sbi::set_timer;
use crate::task::processor::current_processor;
use core::sync::atomic::{AtomicUsize, Ordering};

static TICKS: AtomicUsize = AtomicUsize::new(0);
static KERNEL_TIMER_COUNT: AtomicUsize = AtomicUsize::new(0);
static KERNEL_EXTERNAL_COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn get_ticks() -> usize {
    TICKS.load(Ordering::Relaxed)
}

pub fn get_kernel_timer_count() -> usize {
    KERNEL_TIMER_COUNT.load(Ordering::Relaxed)
}

pub fn get_kernel_external_count() -> usize {
    KERNEL_EXTERNAL_COUNT.load(Ordering::Relaxed)
}

/// Handle timer interrupt (both user and kernel context)
pub fn timer_handler() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    set_next_trigger();
    unsafe {
        current_processor().need_reschedule = true;
    }
}

/// Handle timer in kernel context
pub fn kernel_timer_handler() {
    KERNEL_TIMER_COUNT.fetch_add(1, Ordering::Relaxed);
    timer_handler();
}

/// Handle external interrupt in kernel context
pub fn kernel_external_handler() {
    KERNEL_EXTERNAL_COUNT.fetch_add(1, Ordering::Relaxed);
    handle_external_irq();
}

/// Handle external interrupt (from user trap handler)
pub fn handle_external_irq() {
    let hart_id = get_hart_id();
    let claim_addr = plic_sclaim(hart_id);
    let irq = unsafe { (claim_addr as *const u32).read_volatile() };
    if irq == 0 {
        return;
    }

    match irq {
        UART_IRQ => {
            let ch = crate::sbi::console_getchar();
            if ch >= 0 {
                crate::drivers::uart::push_char(ch as u8);
            }
        }
        VIRTIO0_IRQ => {}
        _ => {
            crate::println!("[kernel] unknown external IRQ: {}", irq);
        }
    }

    // Complete
    unsafe {
        (claim_addr as *mut u32).write_volatile(irq);
    }
}

pub fn set_next_trigger() {
    let time: usize;
    unsafe {
        core::arch::asm!("csrr {}, time", out(reg) time);
    }
    set_timer(time + CLOCK_FREQ / TICKS_PER_SEC);
}

fn get_hart_id() -> usize {
    let id: usize;
    unsafe {
        core::arch::asm!("mv {}, tp", out(reg) id);
    }
    id
}

pub fn plic_init(hart_id: usize) {
    unsafe {
        // Set priorities
        ((PLIC_PRIORITY + UART_IRQ as usize * 4) as *mut u32).write_volatile(1);
        ((PLIC_PRIORITY + VIRTIO0_IRQ as usize * 4) as *mut u32).write_volatile(1);

        // Enable interrupts for this hart
        let senable = plic_senable(hart_id);
        let current = (senable as *const u32).read_volatile();
        (senable as *mut u32).write_volatile(current | (1 << UART_IRQ) | (1 << VIRTIO0_IRQ));

        // Set threshold to 0
        (plic_spriority(hart_id) as *mut u32).write_volatile(0);
    }
}
