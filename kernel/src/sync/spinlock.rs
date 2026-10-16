use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};
/// Lee `RFLAGS`, desactiva las interrupciones en este CPU y devuelve las
/// flags originales.
///
/// # Safety
///
/// Sólo puede llamarse desde anillo 0 y debe cerrarse con
/// [`restore_flags`] **en la misma CPU y en el mismo marco de llamada**: si
/// se devuelve con las interrupciones desactivadas cuando la llamada entró
/// con ellas activas, el CPU queda mudo para siempre. Ese emparejamiento es
/// lo que sostiene el protocolo de ack del shootdown de TLB.
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
/// Restaura `RFLAGS` a partir del valor que [`save_flags_and_cli`] devolvió.
///
/// # Safety
///
/// El `flags` debe ser exactamente el devuelto por
/// [`save_flags_and_cli`] en esta misma CPU; restaurar unas flags ajenas
/// reactivaría las interrupciones dentro de una sección crítica de otro.
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
        loop {
            let flags = unsafe { save_flags_and_cli() };
            if self
                .locked
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return IrqSafeSpinlockGuard { lock: self, flags };
            }
            unsafe {
                restore_flags(flags);
            }
            core::hint::spin_loop();
        }
    }
    pub fn try_lock(&self) -> Option<IrqSafeSpinlockGuard<'_, T>> {
        let flags = unsafe { save_flags_and_cli() };
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(IrqSafeSpinlockGuard { lock: self, flags })
        } else {
            unsafe {
                restore_flags(flags);
            }
            None
        }
    }
    /// Suelta el lock sin pasar por la guarda.
    ///
    /// # Safety
    ///
    /// Sólo desde el mismo CPU que posee la guarda y habiéndola consumido ya
    /// (o garantizando que no volverá a dereferenciarse): libera el `locked`
    /// sin que exista nadie que deba restaurar sus flags.
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
