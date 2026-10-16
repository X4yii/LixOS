use crate::memory::pmm::PAGE_SIZE;
use crate::memory::vma::{Vma, VmaBacking, VmaFlags};
use crate::task::scheduler::with_current_process;
const PROT_READ: u64 = 0x1;
const PROT_WRITE: u64 = 0x2;
const PROT_EXEC: u64 = 0x4;
const MAP_FIXED: u64 = 0x10;
pub fn sys_mmap(addr: u64, len: u64, prot: u64, flags: u64) -> u64 {
    if len == 0 {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    let aligned_len = match len.checked_add(PAGE_SIZE as u64 - 1) {
        Some(sum) => (sum & !(PAGE_SIZE as u64 - 1)) as usize,
        None => return -(lix_sys::ErrorCode::ENOMEM as i32) as u64,
    };
    if aligned_len > 1024 * 1024 * 1024 {
        return -(lix_sys::ErrorCode::ENOMEM as i32) as u64;
    }
    let mut vflags = VmaFlags::USER.union(VmaFlags::ANONYMOUS);
    if prot & PROT_READ != 0 {
        vflags = vflags.union(VmaFlags::READ);
    }
    if prot & PROT_WRITE != 0 {
        vflags = vflags.union(VmaFlags::WRITE);
    }
    if prot & PROT_EXEC != 0 {
        vflags = vflags.union(VmaFlags::EXEC);
    }
    let target_addr = with_current_process(|current| {
        let is_fixed = (flags & MAP_FIXED) != 0;
        let chosen_addr = if addr != 0 && addr % PAGE_SIZE as u64 == 0 {
            let end = addr.checked_add(aligned_len as u64)?;
            if addr >= crate::memory::address::USER_ADDRESS_START
                && end <= crate::memory::address::USER_ADDRESS_END
            {
                let overlap = current.vma_mgr.vmas().iter().any(|v| v.start < end && addr < v.end);
                if !overlap {
                    Some(addr)
                } else if is_fixed {
                    None
                } else {
                    current.vma_mgr.find_free_range(aligned_len)
                }
            } else if is_fixed {
                None
            } else {
                current.vma_mgr.find_free_range(aligned_len)
            }
        } else if is_fixed {
            None
        } else {
            current.vma_mgr.find_free_range(aligned_len)
        }?;
        let vma = Vma::new(
            chosen_addr,
            chosen_addr + aligned_len as u64,
            vflags,
            VmaBacking::Anonymous,
        ).ok()?;
        current.vma_mgr.add_vma(vma).ok()?;
        Some(chosen_addr)
    });
    match target_addr {
        Some(Some(addr)) => addr,
        _ => -(lix_sys::ErrorCode::ENOMEM as i32) as u64,
    }
}
pub fn sys_munmap(addr: u64, len: u64) -> u64 {
    if len == 0 || addr % PAGE_SIZE as u64 != 0 {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    let aligned_len = match len.checked_add(PAGE_SIZE as u64 - 1) {
        Some(sum) => sum & !(PAGE_SIZE as u64 - 1),
        None => return -(lix_sys::ErrorCode::EINVAL as i32) as u64,
    };
    let end = match addr.checked_add(aligned_len) {
        Some(end) if end <= crate::memory::address::USER_ADDRESS_END => end,
        _ => return -(lix_sys::ErrorCode::EINVAL as i32) as u64,
    };
    let result = with_current_process(|current| {
        let mut curr_page = addr;
        while curr_page < end {
            unsafe {
                let _ = crate::memory::vmm::unmap_page(current.pml4_frame, curr_page, true);
            }
            curr_page += PAGE_SIZE as u64;
        }
        current.vma_mgr.unmap_range(addr, end);
        0u64
    });
    result.unwrap_or(-(lix_sys::ErrorCode::ESRCH as i32) as u64)
}
