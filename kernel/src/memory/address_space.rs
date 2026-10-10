use crate::memory::vmm;

/// RAII Guard para espacios de direcciones virtuales de usuarios (PML4).
/// Si ocurre un error antes de `commit()`, el destructor `Drop` destruye automáticamente
/// todas las tablas y marcos físicos mapeados en el espacio de usuario (mitad inferior),
/// garantizando ausencia de fugas de memoria en cargas fallidas de ejecutables ELF.
pub struct AddressSpace {
    pml4_frame: usize,
    committed: bool,
}

impl AddressSpace {
    pub fn new() -> Result<Self, &'static str> {
        let frame = unsafe { vmm::create_userspace_pml4() }?;
        Ok(Self {
            pml4_frame: frame,
            committed: false,
        })
    }

    #[inline]
    pub fn pml4_frame(&self) -> usize {
        self.pml4_frame
    }

    pub fn map_page(
        &self,
        virt_addr: u64,
        phys_addr: u64,
        user: bool,
        writable: bool,
        nx: bool,
    ) -> Result<(), &'static str> {
        unsafe {
            vmm::map_page(self.pml4_frame, virt_addr, phys_addr, user, writable, nx)
        }
    }

    /// Compromete el espacio de direcciones para transferir la propiedad al proceso.
    /// Cancela la destrucción automática en `drop()`.
    pub fn commit(mut self) -> usize {
        self.committed = true;
        self.pml4_frame
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        if !self.committed {
            unsafe {
                vmm::destroy_userspace_pml4(self.pml4_frame);
            }
        }
    }
}
