use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Ticket-based spinlock that disables local interrupts while held.
/// Fair ordering: threads acquire in FIFO order.
pub struct SpinLock<T: ?Sized> {
    next_ticket: AtomicUsize,
    now_serving: AtomicUsize,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Sync for SpinLock<T> {}
unsafe impl<T: ?Sized + Send> Send for SpinLock<T> {}

pub struct SpinLockGuard<'a, T: ?Sized + 'a> {
    lock: &'a SpinLock<T>,
    sie_before: bool,
}

impl<T> SpinLock<T> {
    pub const fn new(data: T) -> Self {
        SpinLock {
            next_ticket: AtomicUsize::new(0),
            now_serving: AtomicUsize::new(0),
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        // Save and disable local interrupts
        let sie_before = is_interrupt_enabled();
        disable_interrupt();

        let ticket = self.next_ticket.fetch_add(1, Ordering::Relaxed);
        while self.now_serving.load(Ordering::Acquire) != ticket {
            core::hint::spin_loop();
        }

        SpinLockGuard {
            lock: self,
            sie_before,
        }
    }

    /// Unsafe: get mutable reference without locking (for init)
    #[allow(dead_code)]
    pub unsafe fn get_mut(&self) -> &mut T {
        &mut *self.data.get()
    }
}

impl<'a, T: ?Sized> Deref for SpinLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T: ?Sized> DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T: ?Sized> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.now_serving.fetch_add(1, Ordering::Release);
        if self.sie_before {
            enable_interrupt();
        }
    }
}

#[inline]
fn is_interrupt_enabled() -> bool {
    let sstatus: usize;
    unsafe {
        core::arch::asm!("csrr {}, sstatus", out(reg) sstatus);
    }
    (sstatus & (1 << 1)) != 0 // SIE bit
}

#[inline]
pub fn disable_interrupt() {
    unsafe {
        core::arch::asm!("csrc sstatus, {}", in(reg) 1usize << 1);
    }
}

#[inline]
pub fn enable_interrupt() {
    unsafe {
        core::arch::asm!("csrs sstatus, {}", in(reg) 1usize << 1);
    }
}
