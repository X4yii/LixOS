use crate::memory::pmm;

pub const MAX_ORDER: usize = 11;

/// Buddy / Multi-Order Physical Frame Allocator
/// Gestiona asignaciones físicas contiguas por órdenes (2^order páginas).
pub struct BuddyAllocator;

impl BuddyAllocator {
    /// Asigna 2^order marcos físicos contiguos.
    /// Retorna el índice del marco inicial o None si no hay memoria contigua suficiente.
    pub fn alloc_order(order: usize) -> Option<usize> {
        if order > MAX_ORDER {
            return None;
        }
        let count = 1 << order;
        pmm::alloc_contiguous_frames(count)
    }

    /// Libera 2^order marcos físicos contiguos a partir del marco inicial `frame`.
    pub fn free_order(frame: usize, order: usize) {
        if order > MAX_ORDER {
            return;
        }
        let count = 1 << order;
        pmm::free_contiguous_frames(frame, count);
    }
}
