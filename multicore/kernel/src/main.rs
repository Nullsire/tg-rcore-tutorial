#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![feature(step_trait)]


extern crate alloc;

use core::arch::global_asm;
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/link_app.S")));

#[macro_use]
mod console;
mod config;
mod drivers;
mod fs;
mod lang;
mod mm;
mod sbi;
mod sync;
mod syscall;
mod task;
mod trap;

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static BOOT_HART_READY: AtomicBool = AtomicBool::new(false);
static HARTS_BOOTED: AtomicUsize = AtomicUsize::new(0);

/// Get app data by name
pub fn get_app_data_by_name(name: &str) -> Option<&'static [u8]> {
    let num = get_num_apps();
    let names = get_app_names();
    for i in 0..num {
        if names[i] == name {
            return Some(get_app_data(i));
        }
    }
    None
}

fn get_num_apps() -> usize {
    extern "C" {
        fn _num_app();
    }
    unsafe { (_num_app as *const () as *const usize).read_volatile() }
}

fn get_app_data(idx: usize) -> &'static [u8] {
    extern "C" {
        fn _num_app();
    }
    let num_app = get_num_apps();
    assert!(idx < num_app);
    let base = _num_app as *const () as *const usize;
    let start = unsafe { base.add(1 + idx).read_volatile() };
    let end = unsafe { base.add(2 + idx).read_volatile() };
    unsafe { core::slice::from_raw_parts(start as *const u8, end - start) }
}

fn get_app_names() -> alloc::vec::Vec<&'static str> {
    extern "C" {
        fn _app_names();
    }
    let num = get_num_apps();
    let mut names = alloc::vec::Vec::new();
    let mut ptr = _app_names as *const () as *const u8;
    for _ in 0..num {
        let mut len = 0;
        unsafe {
            while ptr.add(len).read_volatile() != 0 {
                len += 1;
            }
            let name = core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len));
            names.push(name);
            ptr = ptr.add(len + 1);
        }
    }
    names
}

/// Entry point
/// All harts enter, but only hart 0 (first arg from OpenSBI) does init.
/// Others are parked until signaled.
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
    // a0 = hart_id from OpenSBI
    core::arch::naked_asm!(
        // Save hart_id to tp immediately
        "   mv tp, a0",
        // Set up per-hart stack
        "   la t0, {boot_stack}",
        "   addi t2, a0, 1",
        "   slli t2, t2, {shift}",   // t2 = (hart_id + 1) << 16
        "   add sp, t0, t2",
        // Elect one boot hart via amoswap (atomic)
        "   la t0, {boot_flag}",
        "   li t1, 1",
        ".option push",
        ".option arch, +zaamo",
        "   amoswap.w.aq t2, t1, (t0)",
        ".option pop",
        "   bnez t2, 2f",            // if was already 1, we lost
        // We are the boot hart
        "   j {boot_main}",
        "2:",
        // Secondary hart: spin-wait
        "   la t0, {ready_flag}",
        "3: ld t1, (t0)",
        "   beqz t1, 3b",
        "   fence",
        "   j {secondary_main}",
        boot_stack = sym BOOT_STACK,
        shift = const 16,
        boot_flag = sym BOOT_CLAIMED,
        ready_flag = sym BOOT_HART_READY,
        boot_main = sym boot_main,
        secondary_main = sym secondary_main,
    )
}

#[link_section = ".bss.stack"]
static mut BOOT_STACK: [u8; config::KERNEL_STACK_SIZE * config::MAX_HARTS] =
    [0; config::KERNEL_STACK_SIZE * config::MAX_HARTS];

/// Atomic flag: set to 1 by the boot hart via LR/SC
#[link_section = ".data"]
static BOOT_CLAIMED: AtomicUsize = AtomicUsize::new(0);

fn start_secondary_harts(boot_hart: usize) {
    for hart_id in 0..config::MAX_HARTS {
        if hart_id == boot_hart {
            continue;
        }

        let ret = crate::sbi::hart_start(hart_id, _start as *const () as usize, 0);
        if ret != 0 {
            println!("[kernel] hart_start({}) failed: {}", hart_id, ret);
        } else {
            println!("[kernel] hart_start({}) issued", hart_id);
        }
    }
}

#[no_mangle]
extern "C" fn boot_main() -> ! {
    clear_bss();
    let hart_id = detect_hart_id();

    println!();
    println!("====================================");
    println!("  MultiCore RISC-V OS (Rust)");
    println!("  Booting on {} harts", config::MAX_HARTS);
    println!("  Boot hart: {}", hart_id);
    println!("====================================");
    println!();

    mm::init();
    task::processor::init_processor(hart_id);
    trap::init(hart_id);
    drivers::uart::init();

    start_secondary_harts(hart_id);

    let num_apps = get_num_apps();
    println!("[kernel] found {} user apps", num_apps);
    let names = get_app_names();
    for i in 0..num_apps {
        println!("[kernel]   app {}: {}", i, names[i]);
    }

    let init_app = option_env!("INIT_APP").unwrap_or("shell");
    let init_data = get_app_data_by_name(init_app)
        .or_else(|| get_app_data_by_name("shell"))
        .unwrap_or_else(|| get_app_data(0));
    println!("[kernel] init app: {}", init_app);
    task::init(&[init_data]);

    core::sync::atomic::fence(Ordering::SeqCst);
    BOOT_HART_READY.store(true, Ordering::SeqCst);
    HARTS_BOOTED.fetch_add(1, Ordering::SeqCst);

    println!("[kernel] boot hart {} init complete, entering scheduler", hart_id);
    trap::interrupt::set_next_trigger();
    task::processor::run_tasks();
}

#[no_mangle]
extern "C" fn secondary_main() -> ! {
    let hart_id = detect_hart_id();

    task::processor::init_processor(hart_id);
    trap::init(hart_id);

    let satp = mm::kernel_token();
    unsafe {
        asm!("csrw satp, {}", in(reg) satp);
        asm!("sfence.vma");
    }

    HARTS_BOOTED.fetch_add(1, Ordering::SeqCst);
    println!("[kernel] hart {} online", hart_id);

    trap::interrupt::set_next_trigger();
    task::processor::run_tasks();
}

/// Read current hart_id from tp register (saved in _start)
fn detect_hart_id() -> usize {
    let hart_id: usize;
    unsafe {
        asm!("mv {}, tp", out(reg) hart_id);
    }
    hart_id
}

fn clear_bss() {
    extern "C" {
        fn sbss();
        fn ebss();
    }
    unsafe {
        let start = sbss as *const () as usize;
        let end = ebss as *const () as usize;
        if end > start {
            core::slice::from_raw_parts_mut(start as *mut u8, end - start).fill(0);
        }
    }
}
