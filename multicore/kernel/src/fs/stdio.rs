use super::File;
use crate::sbi;

pub struct Stdin;
pub struct Stdout;

impl File for Stdin {
    fn readable(&self) -> bool {
        true
    }
    fn writable(&self) -> bool {
        false
    }
    fn read(&self, buf: &mut [u8]) -> usize {
        assert_eq!(buf.len(), 1);
        // Try buffered input first, then poll SBI
        loop {
            if let Some(c) = crate::drivers::uart::pop_char() {
                buf[0] = c;
                return 1;
            }
            let c = sbi::console_getchar();
            if c >= 0 {
                buf[0] = c as u8;
                return 1;
            }
            // Yield to prevent busy-waiting
            crate::task::suspend_current_and_run_next();
        }
    }
    fn write(&self, _buf: &[u8]) -> usize {
        panic!("Cannot write to stdin!");
    }
}

impl File for Stdout {
    fn readable(&self) -> bool {
        false
    }
    fn writable(&self) -> bool {
        true
    }
    fn read(&self, _buf: &mut [u8]) -> usize {
        panic!("Cannot read from stdout!");
    }
    fn write(&self, buf: &[u8]) -> usize {
        let _guard = crate::console::lock_console();
        for &c in buf {
            sbi::console_putchar(c);
        }
        buf.len()
    }
}
