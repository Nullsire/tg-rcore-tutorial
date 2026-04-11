use core::arch::asm;

fn syscall(id: usize, args: [usize; 3]) -> isize {
    let mut ret: isize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") args[0] => ret,
            in("a1") args[1],
            in("a2") args[2],
            in("a7") id,
        );
    }
    ret
}

const SYSCALL_READ: usize = 63;
const SYSCALL_WRITE: usize = 64;
const SYSCALL_EXIT: usize = 93;
const SYSCALL_YIELD: usize = 124;
const SYSCALL_GET_TIME: usize = 169;
const SYSCALL_GETPID: usize = 172;
const SYSCALL_FORK: usize = 220;
const SYSCALL_EXEC: usize = 221;
const SYSCALL_WAITPID: usize = 260;
const SYSCALL_THREAD_CREATE: usize = 1000;
const SYSCALL_GETTID: usize = 1001;
const SYSCALL_WAITTID: usize = 1002;
const SYSCALL_MUTEX_CREATE: usize = 1010;
const SYSCALL_MUTEX_LOCK: usize = 1011;
const SYSCALL_MUTEX_UNLOCK: usize = 1012;
const SYSCALL_SEMAPHORE_CREATE: usize = 1020;
const SYSCALL_SEMAPHORE_UP: usize = 1021;
const SYSCALL_SEMAPHORE_DOWN: usize = 1022;
const SYSCALL_SCHED_SET: usize = 1030;
const SYSCALL_HART_ID: usize = 1031;
const SYSCALL_KERNEL_STATS: usize = 1032;
const SYSCALL_KERNEL_IRQ_SET: usize = 1033;
const SYSCALL_ACTIVE_HARTS_SET: usize = 1034;
const SYSCALL_SIG_SEND: usize = 1040;
const SYSCALL_SIG_MASK: usize = 1041;
const SYSCALL_SIG_PENDING: usize = 1042;
const SYSCALL_OPEN: usize = 1050;
const SYSCALL_CLOSE: usize = 1051;
const SYSCALL_LSEEK: usize = 1052;
const SYSCALL_UNLINK: usize = 1053;
const SYSCALL_FSIZE: usize = 1054;

pub fn sys_read(fd: usize, buffer: &mut [u8]) -> isize {
    syscall(SYSCALL_READ, [fd, buffer.as_mut_ptr() as usize, buffer.len()])
}

pub fn sys_write(fd: usize, buffer: &[u8]) -> isize {
    syscall(SYSCALL_WRITE, [fd, buffer.as_ptr() as usize, buffer.len()])
}

pub fn sys_exit(exit_code: i32) -> ! {
    syscall(SYSCALL_EXIT, [exit_code as usize, 0, 0]);
    unreachable!()
}

pub fn sys_yield() -> isize {
    syscall(SYSCALL_YIELD, [0, 0, 0])
}

pub fn sys_get_time() -> isize {
    syscall(SYSCALL_GET_TIME, [0, 0, 0])
}

pub fn sys_getpid() -> isize {
    syscall(SYSCALL_GETPID, [0, 0, 0])
}

pub fn sys_fork() -> isize {
    syscall(SYSCALL_FORK, [0, 0, 0])
}

pub fn sys_exec(path: &str) -> isize {
    syscall(SYSCALL_EXEC, [path.as_ptr() as usize, path.len(), 0])
}

pub fn sys_waitpid(pid: isize, exit_code: &mut i32) -> isize {
    syscall(SYSCALL_WAITPID, [pid as usize, exit_code as *mut i32 as usize, 0])
}

pub fn sys_thread_create(entry: usize, user_sp: usize, arg: usize) -> isize {
    syscall(SYSCALL_THREAD_CREATE, [entry, user_sp, arg])
}

pub fn sys_gettid() -> isize {
    syscall(SYSCALL_GETTID, [0, 0, 0])
}

pub fn sys_waittid(tid: usize) -> isize {
    syscall(SYSCALL_WAITTID, [tid, 0, 0])
}

pub fn sys_mutex_create() -> isize {
    syscall(SYSCALL_MUTEX_CREATE, [0, 0, 0])
}

pub fn sys_mutex_lock(id: usize) -> isize {
    syscall(SYSCALL_MUTEX_LOCK, [id, 0, 0])
}

pub fn sys_mutex_unlock(id: usize) -> isize {
    syscall(SYSCALL_MUTEX_UNLOCK, [id, 0, 0])
}

pub fn sys_semaphore_create(count: usize) -> isize {
    syscall(SYSCALL_SEMAPHORE_CREATE, [count, 0, 0])
}

pub fn sys_semaphore_up(id: usize) -> isize {
    syscall(SYSCALL_SEMAPHORE_UP, [id, 0, 0])
}

pub fn sys_semaphore_down(id: usize) -> isize {
    syscall(SYSCALL_SEMAPHORE_DOWN, [id, 0, 0])
}

pub fn sys_sched_set(stype: usize) -> isize {
    syscall(SYSCALL_SCHED_SET, [stype, 0, 0])
}

pub fn sys_hart_id() -> isize {
    syscall(SYSCALL_HART_ID, [0, 0, 0])
}

pub fn sys_kernel_stats(buf: &mut [u8]) -> isize {
    syscall(SYSCALL_KERNEL_STATS, [buf.as_mut_ptr() as usize, buf.len(), 0])
}

pub fn sys_kernel_irq_set(enabled: usize) -> isize {
    syscall(SYSCALL_KERNEL_IRQ_SET, [enabled, 0, 0])
}

pub fn sys_active_harts_set(count: usize) -> isize {
    syscall(SYSCALL_ACTIVE_HARTS_SET, [count, 0, 0])
}

pub fn sys_sig_send(pid: usize, sig: usize) -> isize {
    syscall(SYSCALL_SIG_SEND, [pid, sig, 0])
}

pub fn sys_sig_mask(new_mask: usize) -> isize {
    syscall(SYSCALL_SIG_MASK, [new_mask, 0, 0])
}

pub fn sys_sig_pending() -> isize {
    syscall(SYSCALL_SIG_PENDING, [0, 0, 0])
}

pub fn sys_open(path: &str, flags: usize) -> isize {
    syscall(SYSCALL_OPEN, [path.as_ptr() as usize, path.len(), flags])
}

pub fn sys_close(fd: usize) -> isize {
    syscall(SYSCALL_CLOSE, [fd, 0, 0])
}

pub fn sys_lseek(fd: usize, offset: isize, whence: usize) -> isize {
    syscall(SYSCALL_LSEEK, [fd, offset as usize, whence])
}

pub fn sys_unlink(path: &str) -> isize {
    syscall(SYSCALL_UNLINK, [path.as_ptr() as usize, path.len(), 0])
}

pub fn sys_fsize(fd: usize, size_ptr: &mut usize) -> isize {
    syscall(SYSCALL_FSIZE, [fd, size_ptr as *mut usize as usize, 0])
}
