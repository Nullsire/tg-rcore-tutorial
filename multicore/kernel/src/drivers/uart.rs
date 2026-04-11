use crate::config::UART_BASE;
use crate::sync::SpinLock;
use alloc::collections::VecDeque;

// NS16550A UART registers
#[allow(dead_code)]
const RBR: usize = 0; // Receive Buffer Register (read)
#[allow(dead_code)]
const THR: usize = 0; // Transmit Holding Register (write)
const IER: usize = 1; // Interrupt Enable Register
const FCR: usize = 2; // FIFO Control Register
const LCR: usize = 3; // Line Control Register
#[allow(dead_code)]
const LSR: usize = 5; // Line Status Register

#[allow(dead_code)]
const LSR_RX_READY: u8 = 1;
#[allow(dead_code)]
const LSR_TX_IDLE: u8 = 1 << 5;

static INPUT_BUFFER: SpinLock<VecDeque<u8>> = SpinLock::new(VecDeque::new());

pub fn init() {
    let base = UART_BASE as *mut u8;
    unsafe {
        // Disable all interrupts
        base.add(IER).write_volatile(0x00);
        // Enable DLAB
        base.add(LCR).write_volatile(0x80);
        // Set baud rate divisor = 3 (38400 baud)
        base.add(0).write_volatile(0x03);
        base.add(1).write_volatile(0x00);
        // 8 bits, no parity, one stop bit
        base.add(LCR).write_volatile(0x03);
        // Enable FIFO
        base.add(FCR).write_volatile(0x07);
        // Enable received data interrupt
        base.add(IER).write_volatile(0x01);
    }
    crate::println!("[kernel] UART initialized");
}

#[allow(dead_code)]
pub fn putc(c: u8) {
    let base = UART_BASE as *mut u8;
    unsafe {
        while base.add(LSR).read_volatile() & LSR_TX_IDLE == 0 {
            core::hint::spin_loop();
        }
        base.add(THR).write_volatile(c);
    }
}

#[allow(dead_code)]
pub fn getc() -> Option<u8> {
    let base = UART_BASE as *mut u8;
    unsafe {
        if base.add(LSR).read_volatile() & LSR_RX_READY != 0 {
            Some(base.add(RBR).read_volatile())
        } else {
            None
        }
    }
}

/// Push a character into the input buffer (called from interrupt handler)
pub fn push_char(c: u8) {
    INPUT_BUFFER.lock().push_back(c);
}

/// Pop a character from the input buffer
pub fn pop_char() -> Option<u8> {
    INPUT_BUFFER.lock().pop_front()
}
