use crate::sbi::console_putchar;
use crate::sync::SpinLock;
use core::fmt::{self, Write};

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.bytes() {
            console_putchar(c);
        }
        Ok(())
    }
}

/// Global console lock shared by both kernel println! and user stdout writes.
/// Prevents character interleaving when multiple harts print simultaneously.
static CONSOLE_LOCK: SpinLock<()> = SpinLock::new(());

/// Acquire the global console lock. Returns a guard that releases on drop.
/// Used by both kernel print and user Stdout::write to ensure atomic output.
pub fn lock_console() -> crate::sync::SpinLockGuard<'static, ()> {
    CONSOLE_LOCK.lock()
}

pub fn print(args: fmt::Arguments) {
    let _guard = CONSOLE_LOCK.lock();
    Stdout.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!($fmt $(, $($arg)+)?))
    }
}

#[macro_export]
macro_rules! println {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
    };
    () => {
        $crate::console::print(format_args!("\n"))
    }
}
