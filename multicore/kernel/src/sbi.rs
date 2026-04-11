use core::arch::asm;

const SBI_SET_TIMER: usize = 0;
const SBI_CONSOLE_PUTCHAR: usize = 1;
const SBI_CONSOLE_GETCHAR: usize = 2;
const SBI_SHUTDOWN: usize = 8;

// SBI HSM extension
const SBI_EXT_HSM: usize = 0x48534D;
const SBI_HSM_HART_START: usize = 0;

// SBI IPI extension
#[allow(dead_code)]
const SBI_EXT_IPI: usize = 0x735049;
#[allow(dead_code)]
const SBI_IPI_SEND: usize = 0;

#[inline(always)]
fn sbi_call(eid: usize, fid: usize, arg0: usize, arg1: usize, arg2: usize) -> (isize, isize) {
    let error: isize;
    let value: isize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") arg0 => error,
            inlateout("a1") arg1 => value,
            in("a2") arg2,
            in("a6") fid,
            in("a7") eid,
        );
    }
    (error, value)
}

pub fn console_putchar(c: u8) {
    sbi_call(SBI_CONSOLE_PUTCHAR, 0, c as usize, 0, 0);
}

pub fn console_getchar() -> isize {
    sbi_call(SBI_CONSOLE_GETCHAR, 0, 0, 0, 0).0
}

pub fn set_timer(timer: usize) {
    sbi_call(SBI_SET_TIMER, 0, timer, 0, 0);
}

pub fn shutdown(failure: bool) -> ! {
    if failure {
        sbi_call(SBI_SHUTDOWN, 0, 1, 0, 0);
    } else {
        sbi_call(SBI_SHUTDOWN, 0, 0, 0, 0);
    }
    unreachable!()
}

pub fn hart_start(hartid: usize, start_addr: usize, opaque: usize) -> isize {
    sbi_call(SBI_EXT_HSM, SBI_HSM_HART_START, hartid, start_addr, opaque).0
}

#[allow(dead_code)]
pub fn send_ipi(hart_mask: usize, hart_mask_base: usize) -> isize {
    sbi_call(SBI_EXT_IPI, SBI_IPI_SEND, hart_mask, hart_mask_base, 0).0
}
