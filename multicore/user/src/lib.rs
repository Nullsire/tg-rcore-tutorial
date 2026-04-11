#![no_std]
#![feature(linkage)]

#[macro_use]
pub mod console;
pub mod syscall;

use syscall::*;

#[no_mangle]
#[link_section = ".text.entry"]
pub extern "C" fn _start() -> ! {
    clear_bss();
    exit(main());
}

#[linkage = "weak"]
#[no_mangle]
fn main() -> i32 {
    panic!("Cannot find main!");
}

fn clear_bss() {
    extern "C" {
        fn start_bss();
        fn end_bss();
    }
    unsafe {
        let start = start_bss as *const () as usize;
        let end = end_bss as *const () as usize;
        if end > start {
            core::slice::from_raw_parts_mut(start as *mut u8, end - start).fill(0);
        }
    }
}

pub fn read(fd: usize, buf: &mut [u8]) -> isize {
    sys_read(fd, buf)
}
pub fn write(fd: usize, buf: &[u8]) -> isize {
    sys_write(fd, buf)
}
pub fn exit(exit_code: i32) -> ! {
    sys_exit(exit_code)
}
pub fn yield_() -> isize {
    sys_yield()
}
pub fn get_time() -> isize {
    sys_get_time()
}
pub fn getpid() -> isize {
    sys_getpid()
}
pub fn fork() -> isize {
    sys_fork()
}
pub fn exec(path: &str) -> isize {
    sys_exec(path)
}
pub fn waitpid(pid: isize, exit_code: &mut i32) -> isize {
    loop {
        match sys_waitpid(pid, exit_code) {
            -2 => {
                yield_();
            }
            n => return n,
        }
    }
}
pub fn wait(exit_code: &mut i32) -> isize {
    waitpid(-1, exit_code)
}
pub fn thread_create(entry: fn(usize) -> i32, arg: usize) -> isize {
    sys_thread_create(entry as usize, 0, arg)
}
pub fn gettid() -> isize {
    sys_gettid()
}
pub fn waittid(tid: usize) -> isize {
    loop {
        match sys_waittid(tid) {
            -2 => {
                yield_();
            }
            n => return n,
        }
    }
}
pub fn hart_id() -> isize {
    sys_hart_id()
}
pub fn sched_set(stype: usize) -> isize {
    sys_sched_set(stype)
}
pub fn kernel_stats(buf: &mut [u8]) -> isize {
    sys_kernel_stats(buf)
}

pub fn kernel_irq_set(enabled: bool) -> isize {
    sys_kernel_irq_set(enabled as usize)
}

pub fn active_harts_set(count: usize) -> isize {
    sys_active_harts_set(count)
}

pub fn sig_send(pid: usize, sig: usize) -> isize {
    sys_sig_send(pid, sig)
}

pub fn sig_mask(new_mask: usize) -> isize {
    sys_sig_mask(new_mask)
}

pub fn sig_pending() -> isize {
    sys_sig_pending()
}

pub const O_CREATE: usize = 1;
pub const O_TRUNC: usize = 2;

pub fn open(path: &str, flags: usize) -> isize {
    sys_open(path, flags)
}

pub fn close(fd: usize) -> isize {
    sys_close(fd)
}

pub fn lseek(fd: usize, offset: isize, whence: usize) -> isize {
    sys_lseek(fd, offset, whence)
}

pub fn unlink(path: &str) -> isize {
    sys_unlink(path)
}

pub fn fsize(fd: usize, size: &mut usize) -> isize {
    sys_fsize(fd, size)
}

pub fn kernel_stats_parsed() -> KernelStats {
    let mut buf = [0u8; 256];
    kernel_stats(&mut buf);
    parse_stats(&buf)
}

pub struct KernelStats {
    pub ticks: usize,
    pub kernel_timer: usize,
    pub kernel_ext: usize,
    pub tasks: usize,
    pub sched: usize,
    pub kirq: usize,
    pub harts: usize,
}

fn parse_stats(buf: &[u8]) -> KernelStats {
    let s = {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        core::str::from_utf8(&buf[..end]).unwrap_or("")
    };
    let mut stats = KernelStats {
        ticks: 0,
        kernel_timer: 0,
        kernel_ext: 0,
        tasks: 0,
        sched: 0,
        kirq: 0,
        harts: 0,
    };
    for field in s.split(',') {
        let mut kv = field.split(':');
        if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
            let val = v.parse::<usize>().unwrap_or(0);
            match k.trim() {
                "ticks" => stats.ticks = val,
                "kernel_timer" => stats.kernel_timer = val,
                "kernel_ext" => stats.kernel_ext = val,
                "tasks" => stats.tasks = val,
                "sched" => stats.sched = val,
                "kirq" => stats.kirq = val,
                "harts" => stats.harts = val,
                _ => {}
            }
        }
    }
    stats
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(location) = info.location() {
        println!(
            "user panic at {}:{}: {}",
            location.file(),
            location.line(),
            info.message()
        );
    } else {
        println!("user panic: {}", info.message());
    }
    exit(-1)
}
