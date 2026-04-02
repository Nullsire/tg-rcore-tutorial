#![no_std]
#![no_main]
#![allow(internal_features)]
#![allow(unsafe_op_in_unsafe_fn)]

#[macro_use]
extern crate user_lib;

extern crate alloc;

use alloc::alloc::{alloc, dealloc, realloc as rust_realloc, Layout};
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::{c_char, c_int, c_void, CStr};

type c_long = i64;
type c_uint = u32;
use core::ptr::null_mut;
use user_lib::{
    close, console_getchar_nonblocking, fb_flush, fb_info, get_time, open, read, sched_yield,
    write,
    OpenFlags, STDOUT,
};

// -----------------------------------------------------------------------------
// Doom Generic Rust Shim
// -----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub static mut DOOMGENERIC_RESX: u32 = 640;
#[unsafe(no_mangle)]
pub static mut DOOMGENERIC_RESY: u32 = 400;

const DOOM_SRC_RESX: usize = 640;
const DOOM_SRC_RESY: usize = 400;

struct StaticCell<T> {
    inner: UnsafeCell<T>,
}

unsafe impl<T> Sync for StaticCell<T> {}

impl<T> StaticCell<T> {
    const fn new(value: T) -> Self {
        Self {
            inner: UnsafeCell::new(value),
        }
    }

    #[inline]
    fn get(&self) -> *mut T {
        self.inner.get()
    }
}

static FB_RESX: StaticCell<usize> = StaticCell::new(DOOM_SRC_RESX);
static FB_RESY: StaticCell<usize> = StaticCell::new(DOOM_SRC_RESY);
static FB_STAGING: StaticCell<Option<Vec<u32>>> = StaticCell::new(None);
static DRAW_DIAG_PRINTED: StaticCell<bool> = StaticCell::new(false);
static TIMER_SYS_LAST_MS: StaticCell<u32> = StaticCell::new(0);
static TIMER_SOFT_MS: StaticCell<u32> = StaticCell::new(0);
static TIMER_STALE_COUNT: StaticCell<u32> = StaticCell::new(0);
static TIMER_DIAG_COUNT: StaticCell<u32> = StaticCell::new(0);
static KEY_LAST_MS: StaticCell<u32> = StaticCell::new(0);
static KEY_DELIVERED_THIS_MS: StaticCell<bool> = StaticCell::new(false);
static KEY_PENDING_CH: StaticCell<i32> = StaticCell::new(-1);
static KEY_PENDING_RELEASE: StaticCell<i32> = StaticCell::new(-1);
static KEY_ESC_STATE: StaticCell<u8> = StaticCell::new(0);

const KEY_RIGHTARROW: u8 = 0xae;
const KEY_LEFTARROW: u8 = 0xac;
const KEY_UPARROW: u8 = 0xad;
const KEY_DOWNARROW: u8 = 0xaf;
const KEY_ESCAPE: u8 = 27;

unsafe extern "C" {
    fn doomgeneric_Create(argc: c_int, argv: *mut *mut c_char);
    fn doomgeneric_Tick();
    static mut DG_ScreenBuffer: *mut u32;
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_Init() {
    let mut w = 0u32;
    let mut h = 0u32;
    if fb_info(&mut w as *mut u32, &mut h as *mut u32) == 0 && w > 0 && h > 0 {
        unsafe {
            *FB_RESX.get() = w as usize;
            *FB_RESY.get() = h as usize;
        }
    }

    unsafe {
        let now = get_time() as u32;
        *TIMER_SYS_LAST_MS.get() = now;
        *TIMER_SOFT_MS.get() = now;
        *TIMER_STALE_COUNT.get() = 0;
        *TIMER_DIAG_COUNT.get() = 0;
        *KEY_LAST_MS.get() = now;
        *KEY_DELIVERED_THIS_MS.get() = false;
        *KEY_PENDING_CH.get() = -1;
        *KEY_PENDING_RELEASE.get() = -1;
        *KEY_ESC_STATE.get() = 0;
        println!("[DG] init fb={}x{} t0={}ms", *FB_RESX.get(), *FB_RESY.get(), now);

        // Draw one test frame so we can verify the display path even if Doom loop stalls.
        let w = *FB_RESX.get();
        let h = *FB_RESY.get();
        let len = w.saturating_mul(h);
        let staging = &mut *FB_STAGING.get();
        *staging = Some(Vec::with_capacity(len));
        if let Some(buf) = staging.as_mut() {
            buf.resize(len, 0);
            for y in 0..h {
                let row = y * w;
                for x in 0..w {
                    let color = if y < h / 3 {
                        0xff0000ff // blue
                    } else if y < (h * 2) / 3 {
                        0xff00ff00 // green
                    } else {
                        0xffff0000 // red
                    };
                    // add a simple white diagonal for orientation
                    let pix = if x == (y * w) / h { 0xffffffff } else { color };
                    buf[row + x] = pix;
                }
            }
            let ret = fb_flush(buf.as_ptr() as *const u8, len * 4);
            println!("[DG] init test-pattern flush={} len={}", ret, len * 4);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_DrawFrame() {
    unsafe {
        if DG_ScreenBuffer.is_null() {
            return;
        }

        let src_w = DOOM_SRC_RESX;
        let src_h = DOOM_SRC_RESY;
        let dst_w = *FB_RESX.get();
        let dst_h = *FB_RESY.get();

        let src_len = src_w * src_h;
        let src = core::slice::from_raw_parts(DG_ScreenBuffer as *const u32, src_len);

        let dst_len = dst_w * dst_h;
        let staging = &mut *FB_STAGING.get();
        if staging.as_ref().map_or(0, |v| v.len()) != dst_len {
            *staging = Some(Vec::with_capacity(dst_len));
            if let Some(buf) = staging.as_mut() {
                buf.resize(dst_len, 0);
            }
        }

        if let Some(dst) = staging.as_mut() {
            // Generic nearest-neighbor scaling to match actual framebuffer size.
            for y in 0..dst_h {
                let sy = (y * src_h) / dst_h;
                let src_row = sy * src_w;
                let dst_row = y * dst_w;
                for x in 0..dst_w {
                    let sx = (x * src_w) / dst_w;
                    // Force opaque alpha to avoid transparent-black output.
                    dst[dst_row + x] = src[src_row + sx] | 0xff00_0000;
                }
            }
            let flush_ret = fb_flush(dst.as_ptr() as *const u8, dst_len * 4);

            let printed = &mut *DRAW_DIAG_PRINTED.get();
            if !*printed {
                let mut nonzero = 0usize;
                for p in src.iter().take(src_len) {
                    if *p != 0 {
                        nonzero += 1;
                    }
                }
                println!(
                    "[DG] draw src={}x{} dst={}x{} nonzero={} flush={}",
                    src_w,
                    src_h,
                    dst_w,
                    dst_h,
                    nonzero,
                    flush_ret,
                );
                *printed = true;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_SleepMs(ms: u32) {
    // Do not rely on kernel wall-clock progress here: if timer interrupts
    // are delayed, a blocking sleep can deadlock TryRunTics -> I_Sleep path.
    // Yield once and advance the software clock used by DG_GetTicksMs.
    if ms > 0 {
        sched_yield();
    }
    unsafe {
        let soft = &mut *TIMER_SOFT_MS.get();
        *soft = soft.saturating_add(ms);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_GetTicksMs() -> u32 {
    unsafe {
        let now = get_time() as u32;
        let sys_last = &mut *TIMER_SYS_LAST_MS.get();
        let soft = &mut *TIMER_SOFT_MS.get();
        let stale = &mut *TIMER_STALE_COUNT.get();
        let diag = &mut *TIMER_DIAG_COUNT.get();

        let ret = if now > *sys_last {
            *sys_last = now;
            if *soft < now {
                *soft = now;
            }
            *stale = 0;
            now
        } else {
            *stale = stale.saturating_add(1);
            if *stale > 256 {
                // clock_gettime appears stalled; use monotonic software fallback
                *soft
            } else {
                *sys_last
            }
        };

        if *diag < 6 {
            println!(
                "[DG] ticks now={} sys_last={} soft={} stale={} ret={}",
                now,
                *sys_last,
                *soft,
                *stale,
                ret,
            );
            *diag += 1;
        }

        ret
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_GetKey(pressed: *mut c_int, key: *mut u8) -> c_int {
    unsafe {
        static DG_GETKEY_ENTER_PRINTED: StaticCell<bool> = StaticCell::new(false);
        static DG_GETKEY_AFTER_CONSOLE_PRINTED: StaticCell<bool> = StaticCell::new(false);

        if !*DG_GETKEY_ENTER_PRINTED.get() {
            println!("[DG] DG_GetKey enter");
            *DG_GETKEY_ENTER_PRINTED.get() = true;
        }

        // Guard against endless polling loops caused by noisy/nonfunctional
        // nonblocking console input: deliver at most one key event per ms.
        let now = DG_GetTicksMs();
        if now != *KEY_LAST_MS.get() {
            *KEY_LAST_MS.get() = now;
            *KEY_DELIVERED_THIS_MS.get() = false;
        } else if *KEY_DELIVERED_THIS_MS.get() {
            return 0;
        }

        if *KEY_PENDING_RELEASE.get() >= 0 {
            *pressed = 0;
            *key = *KEY_PENDING_RELEASE.get() as u8;
            *KEY_PENDING_RELEASE.get() = -1;
            *KEY_DELIVERED_THIS_MS.get() = true;
            return 1;
        }

        let mut ch: i32 = if *KEY_PENDING_CH.get() >= 0 {
            let c = *KEY_PENDING_CH.get();
            *KEY_PENDING_CH.get() = -1;
            c
        } else {
            console_getchar_nonblocking() as i32
        };
        if !*DG_GETKEY_AFTER_CONSOLE_PRINTED.get() {
            println!("[DG] DG_GetKey console={}", ch);
            *DG_GETKEY_AFTER_CONSOLE_PRINTED.get() = true;
        }

        while ch > 0 {
            let c = ch as u8;
            match *KEY_ESC_STATE.get() {
                0 => {
                    if c == KEY_ESCAPE {
                        // Start of possible ANSI escape sequence for arrow keys.
                        *KEY_ESC_STATE.get() = 1;
                        ch = console_getchar_nonblocking() as i32;
                        continue;
                    }
                    let valid = matches!(c, 8 | 9 | 10 | 13)
                        || (32..=126).contains(&c)
                        || matches!(c, b'w' | b'a' | b's' | b'd' | b'W' | b'A' | b'S' | b'D');
                    if valid {
                        *pressed = 1;
                        *key = c;
                        *KEY_PENDING_RELEASE.get() = c as i32;
                        *KEY_DELIVERED_THIS_MS.get() = true;
                        return 1;
                    }
                    return 0;
                }
                1 => {
                    if matches!(c, b'[' | b'O') {
                        *KEY_ESC_STATE.get() = 2;
                        ch = console_getchar_nonblocking() as i32;
                        continue;
                    }
                    // Not an ANSI sequence: treat as plain ESC and keep current char for later.
                    *KEY_ESC_STATE.get() = 0;
                    *KEY_PENDING_CH.get() = ch;
                    *pressed = 1;
                    *key = KEY_ESCAPE;
                    *KEY_PENDING_RELEASE.get() = KEY_ESCAPE as i32;
                    *KEY_DELIVERED_THIS_MS.get() = true;
                    return 1;
                }
                _ => {
                    *KEY_ESC_STATE.get() = 0;
                    let mapped = match c {
                        b'A' => Some(KEY_UPARROW),
                        b'B' => Some(KEY_DOWNARROW),
                        b'C' => Some(KEY_RIGHTARROW),
                        b'D' => Some(KEY_LEFTARROW),
                        _ => None,
                    };
                    if let Some(k) = mapped {
                        *pressed = 1;
                        *key = k;
                        *KEY_PENDING_RELEASE.get() = k as i32;
                        *KEY_DELIVERED_THIS_MS.get() = true;
                        return 1;
                    }
                    return 0;
                }
            }
        }

        if *KEY_ESC_STATE.get() == 1 {
            // Standalone ESC with no trailing bytes available.
            *KEY_ESC_STATE.get() = 0;
            *pressed = 1;
            *key = KEY_ESCAPE;
            *KEY_PENDING_RELEASE.get() = KEY_ESCAPE as i32;
            *KEY_DELIVERED_THIS_MS.get() = true;
            return 1;
        }

        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_SetWindowTitle(_title: *const c_char) {}

// -----------------------------------------------------------------------------
// minimal libc for doom — memory allocator
// -----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn malloc(size: usize) -> *mut c_void {
    if size == 0 {
        return null_mut();
    }
    let layout = Layout::from_size_align(size + 8, 8).unwrap();
    let ptr = unsafe { alloc(layout) };
    if !ptr.is_null() {
        unsafe {
            *(ptr as *mut usize) = size;
            ptr.add(8) as *mut c_void
        }
    } else {
        null_mut()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn free(p: *mut c_void) {
    if p.is_null() {
        return;
    }
    unsafe {
        let ptr = (p as *mut u8).sub(8);
        let size = *(ptr as *mut usize);
        let layout = Layout::from_size_align(size + 8, 8).unwrap();
        dealloc(ptr, layout);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn realloc(p: *mut c_void, new_size: usize) -> *mut c_void {
    if p.is_null() {
        return malloc(new_size);
    }
    if new_size == 0 {
        free(p);
        return null_mut();
    }
    unsafe {
        let ptr = (p as *mut u8).sub(8);
        let old_size = *(ptr as *mut usize);
        let old_layout = Layout::from_size_align(old_size + 8, 8).unwrap();
        let new_ptr = rust_realloc(ptr, old_layout, new_size + 8);
        if !new_ptr.is_null() {
            *(new_ptr as *mut usize) = new_size;
            new_ptr.add(8) as *mut c_void
        } else {
            null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn calloc(nobj: usize, size: usize) -> *mut c_void {
    let total = nobj * size;
    let p = malloc(total);
    if !p.is_null() {
        unsafe {
            core::ptr::write_bytes(p as *mut u8, 0, total);
        }
    }
    p
}

// -----------------------------------------------------------------------------
// minimal libc — printf / fprintf / snprintf / vsnprintf
// On riscv64, variadic C args go in a0-a7 then stack.
// We receive them as fixed usize params — enough for DOOM's usage.
// -----------------------------------------------------------------------------

#[inline]
fn take_arg(args: &[usize], arg_idx: &mut usize) -> usize {
    let arg = if *arg_idx < args.len() { args[*arg_idx] } else { 0 };
    *arg_idx += 1;
    arg
}

#[inline]
fn push_padded(out: &mut Vec<u8>, data: &[u8], width: usize, left_align: bool, pad: u8) {
    if width <= data.len() {
        out.extend_from_slice(data);
        return;
    }
    let pad_count = width - data.len();
    if !left_align {
        for _ in 0..pad_count {
            out.push(pad);
        }
        out.extend_from_slice(data);
    } else {
        out.extend_from_slice(data);
        for _ in 0..pad_count {
            out.push(b' ');
        }
    }
}

fn format_unsigned(mut val: u64, base: u8, upper: bool) -> Vec<u8> {
    if val == 0 {
        let mut single = Vec::new();
        single.push(b'0');
        return single;
    }
    let mut rev = Vec::new();
    while val != 0 {
        let d = (val % base as u64) as u8;
        let ch = match d {
            0..=9 => b'0' + d,
            _ if upper => b'A' + (d - 10),
            _ => b'a' + (d - 10),
        };
        rev.push(ch);
        val /= base as u64;
    }
    rev.reverse();
    rev
}

fn format_c_string(fmt: *const c_char, args: &[usize]) -> Vec<u8> {
    if fmt.is_null() {
        return Vec::new();
    }
    unsafe {
        let fmt_bytes = CStr::from_ptr(fmt).to_bytes();
        let mut out = Vec::new();
        let mut i = 0usize;
        let mut arg_idx = 0usize;

        while i < fmt_bytes.len() {
            if fmt_bytes[i] != b'%' {
                out.push(fmt_bytes[i]);
                i += 1;
                continue;
            }

            i += 1;
            if i >= fmt_bytes.len() {
                out.push(b'%');
                break;
            }
            if fmt_bytes[i] == b'%' {
                out.push(b'%');
                i += 1;
                continue;
            }

            // flags
            let mut left_align = false;
            let mut plus_sign = false;
            let mut space_sign = false;
            let mut alt_form = false;
            let mut zero_pad = false;
            loop {
                if i >= fmt_bytes.len() {
                    break;
                }
                match fmt_bytes[i] {
                    b'-' => left_align = true,
                    b'+' => plus_sign = true,
                    b' ' => space_sign = true,
                    b'#' => alt_form = true,
                    b'0' => zero_pad = true,
                    _ => break,
                }
                i += 1;
            }

            // width
            let mut width = 0usize;
            if i < fmt_bytes.len() && fmt_bytes[i] == b'*' {
                let w = take_arg(args, &mut arg_idx) as isize;
                if w < 0 {
                    left_align = true;
                    width = (-w) as usize;
                } else {
                    width = w as usize;
                }
                i += 1;
            } else {
                while i < fmt_bytes.len() && fmt_bytes[i].is_ascii_digit() {
                    width = width
                        .saturating_mul(10)
                        .saturating_add((fmt_bytes[i] - b'0') as usize);
                    i += 1;
                }
            }

            // precision
            let mut precision: Option<usize> = None;
            if i < fmt_bytes.len() && fmt_bytes[i] == b'.' {
                i += 1;
                if i < fmt_bytes.len() && fmt_bytes[i] == b'*' {
                    let p = take_arg(args, &mut arg_idx) as isize;
                    if p >= 0 {
                        precision = Some(p as usize);
                    }
                    i += 1;
                } else {
                    let mut p = 0usize;
                    while i < fmt_bytes.len() && fmt_bytes[i].is_ascii_digit() {
                        p = p
                            .saturating_mul(10)
                            .saturating_add((fmt_bytes[i] - b'0') as usize);
                        i += 1;
                    }
                    precision = Some(p);
                }
            }

            // length modifier: h/hh/l/ll/z
            let mut len_mod = 0u8; // 0=default, 1=l, 2=ll, 3=z
            if i < fmt_bytes.len() {
                match fmt_bytes[i] {
                    b'l' => {
                        i += 1;
                        len_mod = 1;
                        if i < fmt_bytes.len() && fmt_bytes[i] == b'l' {
                            i += 1;
                            len_mod = 2;
                        }
                    }
                    b'z' => {
                        i += 1;
                        len_mod = 3;
                    }
                    b'h' => {
                        i += 1;
                        if i < fmt_bytes.len() && fmt_bytes[i] == b'h' {
                            i += 1;
                        }
                    }
                    _ => {}
                }
            }

            if i >= fmt_bytes.len() {
                out.push(b'%');
                break;
            }

            let spec = fmt_bytes[i];
            i += 1;

            match spec {
                b's' => {
                    let arg = take_arg(args, &mut arg_idx);
                    let s_ptr = arg as *const c_char;
                    let src = if s_ptr.is_null() {
                        b"(null)".as_slice()
                    } else {
                        CStr::from_ptr(s_ptr).to_bytes()
                    };
                    let used = if let Some(p) = precision {
                        &src[..core::cmp::min(src.len(), p)]
                    } else {
                        src
                    };
                    push_padded(&mut out, used, width, left_align, b' ');
                }
                b'c' => {
                    let arg = take_arg(args, &mut arg_idx);
                    let ch = [arg as u8];
                    push_padded(&mut out, &ch, width, left_align, b' ');
                }
                b'p' => {
                    let arg = take_arg(args, &mut arg_idx);
                    let mut field = Vec::new();
                    field.extend_from_slice(b"0x");
                    field.extend_from_slice(format_unsigned(arg as u64, 16, false).as_slice());
                    push_padded(&mut out, field.as_slice(), width, left_align, b' ');
                }
                b'd' | b'i' | b'u' | b'x' | b'X' | b'o' => {
                    let arg = take_arg(args, &mut arg_idx);

                    let (is_signed, base, upper) = match spec {
                        b'd' | b'i' => (true, 10u8, false),
                        b'u' => (false, 10u8, false),
                        b'x' => (false, 16u8, false),
                        b'X' => (false, 16u8, true),
                        b'o' => (false, 8u8, false),
                        _ => (false, 10u8, false),
                    };

                    let (negative, mut uval) = if is_signed {
                        let sval = match len_mod {
                            1 | 2 => arg as i64,
                            3 => arg as isize as i64,
                            _ => arg as i32 as i64,
                        };
                        if sval < 0 {
                            (true, sval.wrapping_neg() as u64)
                        } else {
                            (false, sval as u64)
                        }
                    } else {
                        let v = match len_mod {
                            1 | 2 => arg as u64,
                            3 => arg as usize as u64,
                            _ => arg as u32 as u64,
                        };
                        (false, v)
                    };

                    let mut digits = format_unsigned(uval, base, upper);

                    if let Some(p) = precision {
                        if p == 0 && uval == 0 {
                            digits.clear();
                        } else if digits.len() < p {
                            let mut pad = Vec::new();
                            pad.resize(p - digits.len(), b'0');
                            pad.extend_from_slice(digits.as_slice());
                            digits = pad;
                        }
                    }

                    let mut prefix = Vec::new();
                    if is_signed {
                        if negative {
                            prefix.push(b'-');
                        } else if plus_sign {
                            prefix.push(b'+');
                        } else if space_sign {
                            prefix.push(b' ');
                        }
                    }

                    if alt_form {
                        match spec {
                            b'x' if uval != 0 => prefix.extend_from_slice(b"0x"),
                            b'X' if uval != 0 => prefix.extend_from_slice(b"0X"),
                            b'o' => {
                                if digits.is_empty() || digits[0] != b'0' {
                                    digits.insert(0, b'0');
                                }
                            }
                            _ => {}
                        }
                    }

                    if zero_pad && !left_align && precision.is_none() {
                        let base_len = prefix.len() + digits.len();
                        if width > base_len {
                            let mut z = Vec::new();
                            z.resize(width - base_len, b'0');
                            z.extend_from_slice(digits.as_slice());
                            digits = z;
                        }
                    }

                    let mut field = Vec::new();
                    field.extend_from_slice(prefix.as_slice());
                    field.extend_from_slice(digits.as_slice());
                    push_padded(&mut out, field.as_slice(), width, left_align, b' ');
                }
                b'%' => out.push(b'%'),
                _ => {
                    out.push(b'%');
                    out.push(spec);
                }
            }
        }

        out
    }
}

unsafe fn read_va_args(ap: *const c_void, max: usize) -> [usize; 8] {
    let mut args = [0usize; 8];
    if ap.is_null() {
        return args;
    }
    let n = core::cmp::min(max, args.len());
    for (i, slot) in args.iter_mut().enumerate().take(n) {
        *slot = *(ap as *const usize).add(i);
    }
    args
}

#[unsafe(no_mangle)]
pub extern "C" fn printf(
    fmt: *const c_char,
    a1: usize, a2: usize, a3: usize, a4: usize, a5: usize,
) -> c_int {
    let out = format_c_string(fmt, &[a1, a2, a3, a4, a5]);
    write(STDOUT, out.as_slice());
    out.len() as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn fprintf(
    _f: *const c_void,
    fmt: *const c_char,
    a1: usize, a2: usize, a3: usize, a4: usize, a5: usize,
) -> c_int {
    let out = format_c_string(fmt, &[a1, a2, a3, a4, a5]);
    write(STDOUT, out.as_slice());
    out.len() as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn vfprintf(
    _f: *const c_void,
    fmt: *const c_char,
    ap: *mut c_void,
) -> c_int {
    let args = unsafe { read_va_args(ap as *const c_void, 8) };
    let out = format_c_string(fmt, &args);
    write(STDOUT, out.as_slice());
    out.len() as c_int
}

/// Simplified snprintf: handles %s, %d, %i, %u, %x, %X, %c, %ld, %lu, %lx
#[unsafe(no_mangle)]
pub extern "C" fn snprintf(
    buf: *mut c_char,
    n: usize,
    fmt: *const c_char,
    a1: usize, a2: usize, a3: usize, a4: usize, a5: usize,
) -> c_int {
    let out = format_c_string(fmt, &[a1, a2, a3, a4, a5]);
    if !buf.is_null() && n > 0 {
        unsafe {
            let copy_len = core::cmp::min(out.len(), n - 1);
            core::ptr::copy_nonoverlapping(out.as_ptr(), buf as *mut u8, copy_len);
            *buf.add(copy_len) = 0;
        }
    }
    out.len() as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn vsnprintf(
    buf: *mut c_char,
    n: usize,
    fmt: *const c_char,
    ap: *mut u8,
) -> c_int {
    let args = unsafe { read_va_args(ap as *const c_void, 8) };
    let out = format_c_string(fmt, &args);
    if !buf.is_null() && n > 0 {
        unsafe {
            let copy_len = core::cmp::min(out.len(), n - 1);
            core::ptr::copy_nonoverlapping(out.as_ptr(), buf as *mut u8, copy_len);
            *buf.add(copy_len) = 0;
        }
    }
    out.len() as c_int
}

// -----------------------------------------------------------------------------
// minimal libc — string / stdlib functions
// -----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn puts(s: *const c_char) -> c_int {
    unsafe {
        if !s.is_null() {
            let st = CStr::from_ptr(s).to_str().unwrap_or("");
            write(STDOUT, st.as_bytes());
            write(STDOUT, b"\n");
        }
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn atoi(s: *const c_char) -> c_int {
    unsafe {
        if s.is_null() {
            return 0;
        }
        let bytes = CStr::from_ptr(s).to_bytes();
        let mut val: i32 = 0;
        let mut neg = false;
        let mut i = 0;
        if i < bytes.len() && bytes[i] == b'-' {
            neg = true;
            i += 1;
        }
        while i < bytes.len() && bytes[i] >= b'0' && bytes[i] <= b'9' {
            val = val * 10 + (bytes[i] - b'0') as i32;
            i += 1;
        }
        if neg { -val } else { val }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn abs(x: c_int) -> c_int {
    if x < 0 { -x } else { x }
}

#[unsafe(no_mangle)]
pub extern "C" fn exit(code: c_int) -> ! {
    user_lib::exit(code);
    unreachable!()
}

#[unsafe(no_mangle)]
pub extern "C" fn atexit(_func: *const c_void) -> c_int {
    0 // no-op
}

#[unsafe(no_mangle)]
pub extern "C" fn remove(_path: *const c_char) -> c_int {
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn rename(_old: *const c_char, _new: *const c_char) -> c_int {
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn mkdir(_path: *const c_char, _mode: u32) -> c_int {
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn I_Endoom() {}

// -----------------------------------------------------------------------------
// C library stubs for functions referenced by doomgeneric code
// -----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub static mut errno: c_int = 0;

#[unsafe(no_mangle)]
pub extern "C" fn getenv(_name: *const c_char) -> *mut c_char {
    null_mut() // no environment
}

#[unsafe(no_mangle)]
pub extern "C" fn isatty(_fd: c_int) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn fileno(_stream: *mut FILE) -> c_int {
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn system(_cmd: *const c_char) -> c_int {
    -1 // no shell
}

#[unsafe(no_mangle)]
pub extern "C" fn sscanf(
    _str: *const c_char,
    _fmt: *const c_char,
    _a1: usize, _a2: usize, _a3: usize,
) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn strdup(s: *const c_char) -> *mut c_char {
    if s.is_null() {
        return null_mut();
    }
    unsafe {
        let bytes = CStr::from_ptr(s).to_bytes();
        let len = bytes.len() + 1;
        let p = malloc(len) as *mut c_char;
        if !p.is_null() {
            core::ptr::copy_nonoverlapping(s as *const u8, p as *mut u8, len);
        }
        p
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn toupper(c: c_int) -> c_int {
    if c >= b'a' as c_int && c <= b'z' as c_int {
        c - 32
    } else {
        c
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tolower(c: c_int) -> c_int {
    if c >= b'A' as c_int && c <= b'Z' as c_int {
        c + 32
    } else {
        c
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isspace(c: c_int) -> c_int {
    let ch = c as u8;
    if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b'\r' || ch == 0x0c || ch == 0x0b {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn isdigit(c: c_int) -> c_int {
    let ch = c as u8;
    if ch >= b'0' && ch <= b'9' { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn isalpha(c: c_int) -> c_int {
    let ch = c as u8;
    if (ch >= b'a' && ch <= b'z') || (ch >= b'A' && ch <= b'Z') { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn putchar(c: c_int) -> c_int {
    let b = c as u8;
    write(STDOUT, &[b]);
    c
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int {
    if s1.is_null() || s2.is_null() {
        return 0;
    }
    let mut a = s1;
    let mut b = s2;
    loop {
        let ca = *a as u8;
        let cb = *b as u8;
        let la = if ca >= b'A' && ca <= b'Z' { ca + 32 } else { ca };
        let lb = if cb >= b'A' && cb <= b'Z' { cb + 32 } else { cb };
        if la != lb || la == 0 {
            return la as c_int - lb as c_int;
        }
        a = a.add(1);
        b = b.add(1);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncasecmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int {
    if s1.is_null() || s2.is_null() || n == 0 {
        return 0;
    }
    let mut a = s1;
    let mut b = s2;
    for _ in 0..n {
        let ca = *a as u8;
        let cb = *b as u8;
        let la = if ca >= b'A' && ca <= b'Z' { ca + 32 } else { ca };
        let lb = if cb >= b'A' && cb <= b'Z' { cb + 32 } else { cb };
        if la != lb || la == 0 {
            return la as c_int - lb as c_int;
        }
        a = a.add(1);
        b = b.add(1);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    if s.is_null() { return 0; }
    let mut len = 0usize;
    while *s.add(len) != 0 {
        len += 1;
    }
    len
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int {
    if s1.is_null() || s2.is_null() { return 0; }
    let mut a = s1;
    let mut b = s2;
    loop {
        if *a != *b || *a == 0 {
            return *a as c_int - *b as c_int;
        }
        a = a.add(1);
        b = b.add(1);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int {
    if s1.is_null() || s2.is_null() || n == 0 { return 0; }
    let mut a = s1;
    let mut b = s2;
    for _ in 0..n {
        if *a != *b || *a == 0 {
            return *a as c_int - *b as c_int;
        }
        a = a.add(1);
        b = b.add(1);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncpy(dst: *mut c_char, src: *const c_char, n: usize) -> *mut c_char {
    if dst.is_null() || src.is_null() { return dst; }
    let mut i = 0usize;
    while i < n && *src.add(i) != 0 {
        *dst.add(i) = *src.add(i);
        i += 1;
    }
    while i < n {
        *dst.add(i) = 0;
        i += 1;
    }
    dst
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    if dst.is_null() || src.is_null() { return dst; }
    let mut i = 0usize;
    while *src.add(i) != 0 {
        *dst.add(i) = *src.add(i);
        i += 1;
    }
    *dst.add(i) = 0;
    dst
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(s: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    if s.is_null() { return s; }
    let mut p = s as *mut u8;
    let v = c as u8;
    let mut i = 0usize;
    while i < n {
        *p = v;
        p = p.add(1);
        i += 1;
    }
    s
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    if dst.is_null() || src.is_null() { return dst; }
    let mut d = dst as *mut u8;
    let mut s = src as *const u8;
    let mut i = 0usize;
    while i < n {
        *d = *s;
        d = d.add(1);
        s = s.add(1);
        i += 1;
    }
    dst
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    if dst.is_null() || src.is_null() { return dst; }
    let d = dst as *mut u8;
    let s = src as *const u8;
    let du = d as usize;
    let su = s as usize;
    if du < su || du >= su.saturating_add(n) {
        let mut i = 0usize;
        while i < n {
            *d.add(i) = *s.add(i);
            i += 1;
        }
    } else {
        let mut i = n;
        while i > 0 {
            i -= 1;
            *d.add(i) = *s.add(i);
        }
    }
    dst
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> c_int {
    if s1.is_null() || s2.is_null() || n == 0 { return 0; }
    let a = s1 as *const u8;
    let b = s2 as *const u8;
    for i in 0..n {
        if *a.add(i) != *b.add(i) {
            return *a.add(i) as c_int - *b.add(i) as c_int;
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strchr(s: *const c_char, c: c_int) -> *mut c_char {
    if s.is_null() { return null_mut(); }
    let ch = c as u8;
    let mut p = s;
    loop {
        if *p as u8 == ch { return p as *mut c_char; }
        if *p == 0 { return null_mut(); }
        p = p.add(1);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strrchr(s: *const c_char, c: c_int) -> *mut c_char {
    if s.is_null() { return null_mut(); }
    let ch = c as u8;
    let mut last: *mut c_char = null_mut();
    let mut p = s;
    loop {
        if *p as u8 == ch { last = p as *mut c_char; }
        if *p == 0 { break; }
        p = p.add(1);
    }
    last
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcat(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    if dst.is_null() || src.is_null() { return dst; }
    let dlen = strlen(dst);
    let mut i = 0usize;
    while *src.add(i) != 0 {
        *dst.add(dlen + i) = *src.add(i);
        i += 1;
    }
    *dst.add(dlen + i) = 0;
    dst
}

// qsort
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qsort(
    base: *mut c_void,
    nmemb: usize,
    size: usize,
    compar: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> c_int>,
) {
    if base.is_null() || nmemb <= 1 || size == 0 || compar.is_none() {
        return;
    }
    let compar = compar.unwrap();
    for i in 1..nmemb {
        let mut j = i;
        while j > 0 {
            let a = (base as *mut u8).add((j - 1) * size) as *const c_void;
            let b = (base as *mut u8).add(j * size) as *const c_void;
            if compar(a, b) > 0 {
                let pa = (base as *mut u8).add((j - 1) * size);
                let pb = (base as *mut u8).add(j * size);
                for k in 0..size {
                    let tmp = *pa.add(k);
                    *pa.add(k) = *pb.add(k);
                    *pb.add(k) = tmp;
                }
                j -= 1;
            } else {
                break;
            }
        }
    }
}

// rand/srand — simple LCG
static mut RAND_SEED: u32 = 1;

#[unsafe(no_mangle)]
pub extern "C" fn rand() -> c_int {
    unsafe {
        RAND_SEED = RAND_SEED.wrapping_mul(1103515245).wrapping_add(12345);
        ((RAND_SEED >> 16) & 0x7fff) as c_int
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn srand(seed: c_uint) {
    unsafe { RAND_SEED = seed; }
}

#[unsafe(no_mangle)]
pub extern "C" fn sleep(seconds: c_uint) -> c_uint {
    if seconds > 0 {
        user_lib::sleep(seconds as usize);
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn usleep(_useconds: c_uint) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn time(_t: *mut c_long) -> c_long {
    let t = get_time() as c_long;
    if !_t.is_null() {
        unsafe { *_t = t; }
    }
    t
}

// -----------------------------------------------------------------------------
// file IO — buffer-based implementation
// -----------------------------------------------------------------------------

#[repr(C)]
pub struct FILE {
    buf: *mut u8,
    size: usize,
    pos: usize,
}

fn read_all_fd(fd: usize) -> (*mut u8, usize) {
    let chunk_size = 4096usize;
    let mut capacity = chunk_size;
    let mut buf = malloc(capacity) as *mut u8;
    if buf.is_null() {
        return (null_mut(), 0);
    }
    let mut total = 0usize;
    loop {
        if total + chunk_size > capacity {
            let new_cap = capacity * 2;
            let new_buf = realloc(buf as *mut c_void, new_cap);
            if new_buf.is_null() {
                free(buf as *mut c_void);
                return (null_mut(), 0);
            }
            buf = new_buf as *mut u8;
            capacity = new_cap;
        }
        let slice = unsafe { core::slice::from_raw_parts_mut(buf.add(total), chunk_size) };
        let r = read(fd, slice);
        if r <= 0 {
            break;
        }
        total += r as usize;
    }
    (buf, total)
}

#[unsafe(no_mangle)]
pub extern "C" fn fopen(filename: *const c_char, mode: *const c_char) -> *mut FILE {
    unsafe {
        let name = CStr::from_ptr(filename).to_str().unwrap_or("");
        let m = CStr::from_ptr(mode).to_str().unwrap_or("r");

        let mut flags = OpenFlags::RDONLY;
        if m.contains('w') {
            flags = OpenFlags::WRONLY | OpenFlags::CREATE;
        } else if m.contains('a') {
            flags = OpenFlags::WRONLY | OpenFlags::CREATE;
        }

        let fd = open(name, flags);
        if fd < 0 {
            return null_mut();
        }

        let (buf, size) = read_all_fd(fd as usize);
        close(fd as usize);

        if buf.is_null() && size > 0 {
            return null_mut();
        }

        let f = malloc(core::mem::size_of::<FILE>()) as *mut FILE;
        if f.is_null() {
            if !buf.is_null() {
                free(buf as *mut c_void);
            }
            return null_mut();
        }
        (*f).buf = buf;
        (*f).size = size;
        (*f).pos = 0;
        f
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn fclose(stream: *mut FILE) -> c_int {
    if stream.is_null() {
        return -1;
    }
    unsafe {
        if !(*stream).buf.is_null() {
            free((*stream).buf as *mut c_void);
        }
        free(stream as *mut c_void);
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn fread(
    ptr: *mut c_void,
    size: usize,
    count: usize,
    stream: *mut FILE,
) -> usize {
    if stream.is_null() || ptr.is_null() || size == 0 {
        return 0;
    }
    unsafe {
        let total = size * count;
        let avail = if (*stream).pos < (*stream).size {
            (*stream).size - (*stream).pos
        } else {
            0
        };
        let to_read = core::cmp::min(total, avail);
        core::ptr::copy_nonoverlapping((*stream).buf.add((*stream).pos), ptr as *mut u8, to_read);
        (*stream).pos += to_read;
        to_read / size
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn fwrite(
    ptr: *const c_void,
    size: usize,
    count: usize,
    stream: *mut FILE,
) -> usize {
    let _ = (ptr, size, count, stream);
    0
}

const SEEK_SET: c_int = 0;
const SEEK_CUR: c_int = 1;
const SEEK_END: c_int = 2;

#[unsafe(no_mangle)]
pub extern "C" fn fseek(stream: *mut FILE, offset: c_long, whence: c_int) -> c_int {
    if stream.is_null() {
        return -1;
    }
    unsafe {
        let new_pos = match whence {
            SEEK_SET => offset as isize,
            SEEK_CUR => (*stream).pos as isize + offset as isize,
            SEEK_END => (*stream).size as isize + offset as isize,
            _ => return -1,
        };
        if new_pos < 0 || new_pos as usize > (*stream).size {
            return -1;
        }
        (*stream).pos = new_pos as usize;
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ftell(stream: *mut FILE) -> c_long {
    if stream.is_null() {
        return -1;
    }
    unsafe { (*stream).pos as c_long }
}

#[unsafe(no_mangle)]
pub extern "C" fn feof(stream: *mut FILE) -> c_int {
    if stream.is_null() {
        return 1;
    }
    unsafe {
        if (*stream).pos >= (*stream).size {
            1
        } else {
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ferror(_stream: *mut FILE) -> c_int {
    0
}

static mut STDERR: usize = 2;
static mut STDOUT_VAL: usize = 1;

#[unsafe(no_mangle)]
pub static mut stderr: *mut usize = unsafe { &raw mut STDERR };
#[unsafe(no_mangle)]
pub static mut stdout: *mut usize = unsafe { &raw mut STDOUT_VAL };

#[unsafe(no_mangle)]
pub extern "C" fn fflush(_stream: *mut FILE) -> c_int {
    0
}

// -----------------------------------------------------------------------------
// entry
// -----------------------------------------------------------------------------

/// Read current stack pointer
#[inline(always)]
fn get_sp() -> usize {
    let sp: usize;
    unsafe { core::arch::asm!("mv {}, sp", out(reg) sp) };
    sp
}

/// Format hex number to buffer (no alloc, no fmt)
fn hex_to_buf(val: usize, buf: &mut [u8; 18]) -> &[u8] {
    buf[0] = b'0'; buf[1] = b'x';
    if val == 0 {
        buf[2] = b'0';
        return &buf[..3];
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut i = 2;
    let mut shift = (core::mem::size_of::<usize>() as u32 * 8 - 4) as usize;
    while shift > 0 && (val >> shift) & 0xf == 0 { shift -= 4; }
    loop {
        buf[i] = HEX[(val >> shift) & 0xf];
        i += 1;
        if shift == 0 { break; }
        shift -= 4;
    }
    &buf[..i]
}

#[unsafe(no_mangle)]
pub extern "C" fn main(_argc: c_int, _argv: *mut *mut c_char) -> c_int {
    let mut hbuf = [0u8; 18];
    write(STDOUT, b"[DIAG] doom main() sp=");
    write(STDOUT, hex_to_buf(get_sp(), &mut hbuf));
    write(STDOUT, b"\n");

    let argv0 = b"doom\0";
    let argv1 = b"-iwad\0";
    let argv2 = b"doom1.wad\0";
    // Use static C strings directly to avoid unnecessary heap copies at startup.
    let mut args: [*mut c_char; 3] = [
        argv0.as_ptr() as *mut c_char,
        argv1.as_ptr() as *mut c_char,
        argv2.as_ptr() as *mut c_char,
    ];

    write(STDOUT, b"[DIAG] argv ready sp=");
    write(STDOUT, hex_to_buf(get_sp(), &mut hbuf));
    write(STDOUT, b"\n");

    write(STDOUT, b"[DIAG] before Create sp=");
    write(STDOUT, hex_to_buf(get_sp(), &mut hbuf));
    write(STDOUT, b"\n");
    unsafe {
        doomgeneric_Create(3, args.as_mut_ptr());
    }
    write(STDOUT, b"[DIAG] after Create\n");

    unsafe {
        loop {
            doomgeneric_Tick();
        }
    }
}
