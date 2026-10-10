//! Canonical x86_64 address types and the kernel virtual-address layout.
//!
//! Physical addresses are never interchangeable with kernel virtual addresses.
//! Conversions that can overflow or leave the canonical address space fail.

use core::fmt;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PhysAddr(u64);

impl PhysAddr {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct VirtAddr(u64);

impl VirtAddr {
    pub fn new(value: u64) -> Result<Self, AddressError> {
        if is_canonical(value) {
            Ok(Self(value))
        } else {
            Err(AddressError::NonCanonicalVirtualAddress(value))
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HhdmOffset(u64);

impl HhdmOffset {
    pub fn new(value: u64) -> Result<Self, AddressError> {
        if value & 0xFFF != 0 || !is_canonical(value) {
            return Err(AddressError::InvalidHhdmOffset(value));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn map_phys(self, physical: PhysAddr) -> Result<VirtAddr, AddressError> {
        let value = self.0.checked_add(physical.0)
            .ok_or(AddressError::AddressOverflow)?;
        VirtAddr::new(value)
    }

    pub fn unmap_virt(self, virtual_address: VirtAddr) -> Result<PhysAddr, AddressError> {
        let physical = virtual_address.0.checked_sub(self.0)
            .ok_or(AddressError::OutsideHhdm(virtual_address.0))?;
        Ok(PhysAddr::new(physical))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressError {
    InvalidHhdmOffset(u64),
    NonCanonicalVirtualAddress(u64),
    AddressOverflow,
    OutsideHhdm(u64),
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHhdmOffset(value) => write!(f, "invalid HHDM offset {value:#x}"),
            Self::NonCanonicalVirtualAddress(value) => write!(f, "non-canonical virtual address {value:#x}"),
            Self::AddressOverflow => f.write_str("address conversion overflow"),
            Self::OutsideHhdm(value) => write!(f, "virtual address {value:#x} is outside HHDM"),
        }
    }
}

#[inline]
pub const fn is_canonical(value: u64) -> bool {
    let upper = value >> 48;
    let sign = (value >> 47) & 1;
    (sign == 0 && upper == 0) || (sign == 1 && upper == 0xFFFF)
}

/// Returns the HHDM offset supplied by Limine. Missing boot information is fatal:
/// using a guessed offset can turn physical-memory access into arbitrary memory access.
pub fn hhdm_offset() -> HhdmOffset {
    let response = crate::HHDM_REQUEST
        .get_response()
        .unwrap_or_else(|| panic!("Limine did not provide the required HHDM response"));
    HhdmOffset::new(response.offset())
        .unwrap_or_else(|error| panic!("Limine returned an invalid HHDM offset: {error}"))
}

pub const USER_ADDRESS_START: u64 = 0x0000_0000_0040_0000;
pub const USER_ADDRESS_END: u64 = 0x0000_8000_0000_0000;

/// Reserved higher-half regions. Device mappings must use the MMIO mapping API,
/// not the HHDM or heap ranges.
pub mod layout {
    pub const HHDM_START: u64 = 0xFFFF_8000_0000_0000;
    pub const HHDM_RESERVED_END: u64 = 0xFFFF_C000_0000_0000;
    pub const KERNEL_HEAP_START: u64 = 0xFFFF_C000_0000_0000;
    pub const MMIO_START: u64 = 0xFFFF_D000_0000_0000;
    pub const KERNEL_STACKS_START: u64 = 0xFFFF_E000_0000_0000;
    pub const KERNEL_IMAGE_START: u64 = 0xFFFF_FFFF_8000_0000;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_boundary_matches_four_level_paging() {
        assert!(is_canonical(0x0000_7FFF_FFFF_FFFF));
        assert!(!is_canonical(0x0000_8000_0000_0000));
        assert!(is_canonical(0xFFFF_8000_0000_0000));
        assert!(!is_canonical(0xFFFF_7FFF_FFFF_FFFF));
    }

    #[test]
    fn hhdm_conversion_round_trips() {
        let hhdm = HhdmOffset::new(0xFFFF_8000_0000_0000).unwrap();
        let physical = PhysAddr::new(0x1234_5000);
        let virtual_address = hhdm.map_phys(physical).unwrap();
        assert_eq!(hhdm.unmap_virt(virtual_address).unwrap(), physical);
    }

    #[test]
    fn hhdm_rejects_noncanonical_result() {
        let hhdm = HhdmOffset::new(0xFFFF_8000_0000_0000).unwrap();
        assert!(hhdm.map_phys(PhysAddr::new(0x0000_8000_0000_0000)).is_err());
    }
}
