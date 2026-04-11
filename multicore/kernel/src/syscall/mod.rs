mod fs;
mod process;
mod thread;
mod sync_calls;

// Syscall numbers
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

pub fn syscall(syscall_id: usize, args: [usize; 3]) -> isize {
    match syscall_id {
        SYSCALL_READ => fs::sys_read(args[0], args[1] as *const u8, args[2]),
        SYSCALL_WRITE => fs::sys_write(args[0], args[1] as *const u8, args[2]),
        SYSCALL_OPEN => fs::sys_open(args[0] as *const u8, args[1], args[2]),
        SYSCALL_CLOSE => fs::sys_close(args[0]),
        SYSCALL_LSEEK => fs::sys_lseek(args[0], args[1] as isize, args[2]),
        SYSCALL_UNLINK => fs::sys_unlink(args[0] as *const u8, args[1]),
        SYSCALL_FSIZE => fs::sys_fsize(args[0], args[1] as *mut usize),
        SYSCALL_EXIT => process::sys_exit(args[0] as i32),
        SYSCALL_YIELD => process::sys_yield(),
        SYSCALL_GET_TIME => process::sys_get_time(),
        SYSCALL_GETPID => process::sys_getpid(),
        SYSCALL_FORK => process::sys_fork(),
        SYSCALL_EXEC => process::sys_exec(args[0] as *const u8, args[1]),
        SYSCALL_WAITPID => process::sys_waitpid(args[0] as isize, args[1] as *mut i32),
        SYSCALL_THREAD_CREATE => thread::sys_thread_create(args[0], args[1], args[2]),
        SYSCALL_GETTID => thread::sys_gettid(),
        SYSCALL_WAITTID => thread::sys_waittid(args[0]),
        SYSCALL_MUTEX_CREATE => sync_calls::sys_mutex_create(),
        SYSCALL_MUTEX_LOCK => sync_calls::sys_mutex_lock(args[0]),
        SYSCALL_MUTEX_UNLOCK => sync_calls::sys_mutex_unlock(args[0]),
        SYSCALL_SEMAPHORE_CREATE => sync_calls::sys_semaphore_create(args[0]),
        SYSCALL_SEMAPHORE_UP => sync_calls::sys_semaphore_up(args[0]),
        SYSCALL_SEMAPHORE_DOWN => sync_calls::sys_semaphore_down(args[0]),
        SYSCALL_SCHED_SET => process::sys_sched_set(args[0]),
        SYSCALL_HART_ID => process::sys_hart_id(),
        SYSCALL_KERNEL_STATS => process::sys_kernel_stats(args[0] as *mut u8, args[1]),
        SYSCALL_KERNEL_IRQ_SET => process::sys_kernel_irq_set(args[0]),
        SYSCALL_ACTIVE_HARTS_SET => process::sys_active_harts_set(args[0]),
        SYSCALL_SIG_SEND => process::sys_sig_send(args[0], args[1]),
        SYSCALL_SIG_MASK => process::sys_sig_mask(args[0]),
        SYSCALL_SIG_PENDING => process::sys_sig_pending(),
        _ => {
            crate::println!("[kernel] unsupported syscall: {}", syscall_id);
            -1
        }
    }
}
