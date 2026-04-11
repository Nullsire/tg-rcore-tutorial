use crate::task;
use crate::task::processor::{current_task, current_user_token, hart_id};
use crate::task::scheduler::{SchedulerType, set_scheduler};
use crate::mm::{translated_refmut, translated_byte_buffer};
use crate::trap::interrupt;
use core::fmt::Write;
use alloc::string::String;

fn translated_string(token: usize, ptr: *const u8, len: usize) -> Option<String> {
    if ptr.is_null() || len == 0 {
        return None;
    }
    let mut bytes = alloc::vec::Vec::with_capacity(len);
    for chunk in translated_byte_buffer(token, ptr, len) {
        bytes.extend_from_slice(chunk);
    }
    core::str::from_utf8(&bytes).ok().map(String::from)
}

pub fn sys_exit(exit_code: i32) -> isize {
    task::exit_current_and_run_next(exit_code);
    0 // unreachable
}

pub fn sys_yield() -> isize {
    task::suspend_current_and_run_next();
    0
}

pub fn sys_get_time() -> isize {
    let time: usize;
    unsafe {
        core::arch::asm!("csrr {}, time", out(reg) time);
    }
    (time / (crate::config::CLOCK_FREQ / 1000)) as isize
}

pub fn sys_getpid() -> isize {
    let task = current_task().unwrap();
    let process = task.get_process();
    process.getpid() as isize
}

pub fn sys_fork() -> isize {
    let task = current_task().unwrap();
    let process = task.get_process();
    let child = process.fork();
    let pid = child.getpid();
    task::add_process(child);
    pid as isize
}

pub fn sys_exec(path: *const u8, len: usize) -> isize {
    let token = current_user_token();
    let Some(path) = translated_string(token, path, len) else {
        return -1;
    };
    // Look up the app by name in embedded apps
    if let Some(data) = crate::get_app_data_by_name(&path) {
        let task = current_task().unwrap();
        let process = task.get_process();
        process.exec(data);
        0
    } else {
        -1
    }
}

pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    let task = current_task().unwrap();
    let process = task.get_process();

    loop {
        // Clone the candidate children while holding the process lock, then
        // inspect each child without keeping the process lock held. This avoids
        // ABBA deadlock with the exit path, which locks child state before
        // taking the process lock.
        let children = {
            let inner = process.inner.lock();
            if pid == -1 {
                inner.children.clone()
            } else {
                inner
                    .children
                    .iter()
                    .filter(|child| child.getpid() == pid as usize)
                    .cloned()
                    .collect::<alloc::vec::Vec<_>>()
            }
        };

        if children.is_empty() {
            return -1; // no such child
        }

        for child in children {
            let child_inner = child.inner.lock();
            if child_inner.is_zombie {
                let exit_code = child_inner.exit_code;
                let child_pid = child.getpid();
                drop(child_inner);

                if !exit_code_ptr.is_null() {
                    let token = current_user_token();
                    *translated_refmut(token, exit_code_ptr) = exit_code;
                }

                let mut inner = process.inner.lock();
                if let Some(idx) = inner.children.iter().position(|c| c.getpid() == child_pid) {
                    inner.children.remove(idx);
                }
                // Remove zombie from global process list to free resources
                drop(inner);
                task::remove_process(child_pid);
                return child_pid as isize;
            }
        }

        // Child not yet exited; yield and try again.
        task::suspend_current_and_run_next();
    }
}

pub fn sys_sched_set(stype: usize) -> isize {
    match stype {
        0 => {
            set_scheduler(SchedulerType::GlobalRoundRobin);
            0
        }
        1 => {
            set_scheduler(SchedulerType::PerCpuWorkStealing);
            0
        }
        _ => -1,
    }
}

pub fn sys_hart_id() -> isize {
    hart_id() as isize
}

pub fn sys_kernel_irq_set(enabled: usize) -> isize {
    crate::trap::set_kernel_irq_enabled(enabled != 0);
    0
}

pub fn sys_active_harts_set(count: usize) -> isize {
    if crate::task::processor::set_active_harts(count) {
        0
    } else {
        -1
    }
}

pub fn sys_sig_send(pid: usize, sig: usize) -> isize {
    crate::task::send_signal(pid, sig)
}

pub fn sys_sig_mask(new_mask: usize) -> isize {
    let task = current_task().unwrap();
    let process = task.get_process();
    let mut inner = process.inner.lock();
    let old = inner.signal_mask;
    inner.signal_mask = new_mask;
    old as isize
}

pub fn sys_sig_pending() -> isize {
    let task = current_task().unwrap();
    let process = task.get_process();
    let inner = process.inner.lock();
    inner.signal_pending as isize
}

/// Write kernel statistics to a user buffer
pub fn sys_kernel_stats(buf: *mut u8, len: usize) -> isize {
    let token = current_user_token();
    struct StatsBuf {
        buf: [u8; 160],
        len: usize,
    }

    impl StatsBuf {
        fn new() -> Self {
            Self { buf: [0; 160], len: 0 }
        }

        fn as_bytes(&self) -> &[u8] {
            &self.buf[..self.len]
        }
    }

    impl Write for StatsBuf {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let remaining = self.buf.len().saturating_sub(self.len);
            if s.len() > remaining {
                return Err(core::fmt::Error);
            }
            let end = self.len + s.len();
            self.buf[self.len..end].copy_from_slice(s.as_bytes());
            self.len = end;
            Ok(())
        }
    }

    let mut stats = StatsBuf::new();
    let _ = write!(
        &mut stats,
        "ticks:{},kernel_timer:{},kernel_ext:{},tasks:{},sched:{},kirq:{},harts:{}",
        interrupt::get_ticks(),
        interrupt::get_kernel_timer_count(),
        interrupt::get_kernel_external_count(),
        crate::task::scheduler::task_count(),
        crate::task::scheduler::get_scheduler_type() as usize,
        crate::trap::is_kernel_irq_enabled() as usize,
        crate::task::processor::active_harts(),
    );

    let bytes = stats.as_bytes();
    let copy_len = bytes.len().min(len);
    let buffers = translated_byte_buffer(token, buf as *const u8, copy_len);
    let mut offset = 0;
    for buffer in buffers {
        let chunk = &bytes[offset..offset + buffer.len()];
        buffer.copy_from_slice(chunk);
        offset += buffer.len();
    }
    copy_len as isize
}
