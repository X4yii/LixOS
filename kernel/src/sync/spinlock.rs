use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

#[inline(always)]
unsafe fn save_flags_and_cli() -> usize {
    let flags: usize;
    core::arch::asm!(
        "pushfq",
        "pop {}",
        "cli",
        out(reg) flags,
        options(nomem, nostack, preserves_flags)
    );
    flags
}

#[inline(always)]
unsafe fn restore_flags(flags: usize) {
    if (flags & (1 << 9)) != 0 {
        core::arch::asm!("sti", options(nomem, nostack));
    }
}

pub struct IrqSafeSpinlock<T: ?Sized> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Sync for IrqSafeSpinlock<T> {}
unsafe impl<T: ?Sized + Send> Send for IrqSafeSpinlock<T> {}

impl<T> IrqSafeSpinlock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }
}

impl<T: ?Sized> IrqSafeSpinlock<T> {
    pub fn lock(&self) -> IrqSafeSpinlockGuard<'_, T> {
        let flags = unsafe { save_flags_and_cli() };
        while self.locked.compare_exchange_weak(
            false,
            true,
            Ordering::Acquire,
            Ordering::Relaxed,
        ).is_err() {
            core::hint::spin_loop();
        }

        IrqSafeSpinlockGuard { lock: self, flags }
    }

    pub fn try_lock(&self) -> Option<IrqSafeSpinlockGuard<'_, T>> {
        let flags = unsafe { save_flags_and_cli() };
        if self.locked.compare_exchange(
            false,
            true,
            Ordering::Acquire,
            Ordering::Relaxed,
        ).is_ok() {
            Some(IrqSafeSpinlockGuard { lock: self, flags })
        } else {
            unsafe { restore_flags(flags); }
            None
        }
    }
    pub unsafe fn force_unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

pub struct IrqSafeSpinlockGuard<'a, T: ?Sized> {
    lock: &'a IrqSafeSpinlock<T>,
    flags: usize,
}

impl<'a, T: ?Sized> Deref for IrqSafeSpinlockGuard<'a, T> {
    type Target = T;
    #[inline(always)]
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T: ?Sized> DerefMut for IrqSafeSpinlockGuard<'a, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T: ?Sized> Drop for IrqSafeSpinlockGuard<'a, T> {
    #[inline(always)]
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        unsafe {
            restore_flags(self.flags);
        }
    }
}