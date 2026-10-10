use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use lix_sys::{ErrorCode, Handle, Rights};
use crate::task::process::ProcessId;
use core::sync::atomic::{AtomicBool, Ordering};

static PRIMARY_BLOCK_LEASED: AtomicBool = AtomicBool::new(false);

/// Exclusive capability for the currently enumerated primary block device.
/// The lease lasts until the last owning handle is closed or its process exits.
#[derive(Debug)]
pub struct BlockDeviceCapability {
    pub device_index: u32,
    pub writable: bool,
}

impl BlockDeviceCapability {
    pub fn acquire(device_index: u32, writable: bool) -> Result<Arc<Self>, ErrorCode> {
        if device_index != 0 {
            return Err(ErrorCode::ENODEV);
        }
        PRIMARY_BLOCK_LEASED
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| ErrorCode::EBUSY)?;
        Ok(Arc::new(Self { device_index, writable }))
    }
}

impl Drop for BlockDeviceCapability {
    fn drop(&mut self) {
        PRIMARY_BLOCK_LEASED.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone)]
pub enum KernelObject {
    StdIn,
    StdOut,
    StdErr,
    File {
        path: String,
        cursor: u64,
        is_write: bool,
    },
    Process(ProcessId),
    BlockDevice(Arc<BlockDeviceCapability>),
}

#[derive(Debug, Clone)]
pub struct HandleEntry {
    pub object: KernelObject,
    pub rights: Rights,
    pub generation: u32,
}

pub struct HandleTable {
    slots: Vec<Option<HandleEntry>>,
    generations: Vec<u32>,
}

impl HandleTable {
    pub fn new() -> Self {
        let mut table = Self {
            slots: Vec::with_capacity(16),
            generations: Vec::with_capacity(16),
        };

        table.insert_fixed(
            0,
            KernelObject::StdIn,
            Rights(Rights::READ.0 | Rights::WAIT.0 | Rights::DUP.0),
        );
        table.insert_fixed(
            1,
            KernelObject::StdOut,
            Rights(Rights::WRITE.0 | Rights::DUP.0),
        );
        table.insert_fixed(
            2,
            KernelObject::StdErr,
            Rights(Rights::WRITE.0 | Rights::DUP.0),
        );

        table
    }

    fn insert_fixed(&mut self, idx: usize, object: KernelObject, rights: Rights) -> Handle {
        while self.slots.len() <= idx {
            self.slots.push(None);
            self.generations.push(1);
        }
        let gen = self.generations[idx];
        self.slots[idx] = Some(HandleEntry {
            object,
            rights,
            generation: gen,
        });
        Handle::new(idx as u32, gen)
    }

    pub fn insert(&mut self, object: KernelObject, rights: Rights) -> Result<Handle, ErrorCode> {
        for idx in 0..self.slots.len() {
            if self.slots[idx].is_none() {
                let gen = self.generations[idx];
                self.slots[idx] = Some(HandleEntry {
                    object,
                    rights,
                    generation: gen,
                });
                return Ok(Handle::new(idx as u32, gen));
            }
        }

        if self.slots.len() >= 4096 {
            return Err(ErrorCode::EMFILE);
        }

        let idx = self.slots.len();
        self.slots.push(None);
        self.generations.push(1);
        let gen = 1;
        self.slots[idx] = Some(HandleEntry {
            object,
            rights,
            generation: gen,
        });
        Ok(Handle::new(idx as u32, gen))
    }

    pub fn get(&self, handle: Handle, required_rights: Rights) -> Result<&KernelObject, ErrorCode> {
        let idx = handle.index() as usize;
        let gen = handle.generation();

        if gen == 0 && idx < 3 && idx < self.slots.len() {
            if let Some(ref entry) = self.slots[idx] {
                if entry.rights.contains(required_rights) {
                    return Ok(&entry.object);
                } else {
                    return Err(ErrorCode::EACCES);
                }
            }
        }

        if idx >= self.slots.len() {
            return Err(ErrorCode::EBADF);
        }

        match &self.slots[idx] {
            Some(entry) => {
                if entry.generation != gen {
                    return Err(ErrorCode::EBADF);
                }
                if !entry.rights.contains(required_rights) {
                    return Err(ErrorCode::EACCES);
                }
                Ok(&entry.object)
            }
            None => Err(ErrorCode::EBADF),
        }
    }

    pub fn close(&mut self, handle: Handle) -> Result<(), ErrorCode> {
        let idx = handle.index() as usize;
        let gen = handle.generation();

        if idx >= self.slots.len() {
            return Err(ErrorCode::EBADF);
        }

        if let Some(ref entry) = self.slots[idx] {
            if gen != 0 && entry.generation != gen {
                return Err(ErrorCode::EBADF);
            }
            self.slots[idx] = None;
            self.generations[idx] = self.generations[idx].wrapping_add(1).max(1);
            Ok(())
        } else {
            Err(ErrorCode::EBADF)
        }
    }

    pub fn dup(&mut self, handle: Handle, requested_rights: Rights) -> Result<Handle, ErrorCode> {
        let entry = {
            let idx = handle.index() as usize;
            if idx >= self.slots.len() {
                return Err(ErrorCode::EBADF);
            }
            match &self.slots[idx] {
                Some(e) => {
                    if handle.generation() != 0 && e.generation != handle.generation() {
                        return Err(ErrorCode::EBADF);
                    }
                    if !e.rights.contains(Rights::DUP) {
                        return Err(ErrorCode::EPERM);
                    }
                    let final_rights = if requested_rights.0 == 0 {
                        e.rights
                    } else {
                        e.rights.intersection(requested_rights)
                    };
                    HandleEntry {
                        object: e.object.clone(),
                        rights: final_rights,
                        generation: 0,
                    }
                }
                None => return Err(ErrorCode::EBADF),
            }
        };

        self.insert(entry.object, entry.rights)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_device_lease_is_exclusive_and_released_on_drop() {
        let first = match BlockDeviceCapability::acquire(0, false) {
            Ok(capability) => capability,
            Err(_) => panic!("first lease was unexpectedly unavailable"),
        };
        assert!(matches!(
            BlockDeviceCapability::acquire(0, true),
            Err(ErrorCode::EBUSY)
        ));
        drop(first);
        let second = match BlockDeviceCapability::acquire(0, true) {
            Ok(capability) => capability,
            Err(_) => panic!("lease was not released on drop"),
        };
        assert!(second.writable);
    }

    #[test]
    fn lease_rejects_unknown_device_indices() {
        assert!(matches!(
            BlockDeviceCapability::acquire(1, false),
            Err(ErrorCode::ENODEV)
        ));
    }

    #[test]
    fn block_handle_rights_and_close_control_the_lease() {
        let capability = match BlockDeviceCapability::acquire(0, false) {
            Ok(capability) => capability,
            Err(_) => panic!("lease acquisition failed"),
        };
        let mut handles = HandleTable::new();
        let handle = handles
            .insert(
                KernelObject::BlockDevice(capability),
                Rights::READ.union(Rights::INSPECT),
            )
            .unwrap_or_else(|_| panic!("handle insertion failed"));
        assert!(matches!(handles.get(handle, Rights::WRITE), Err(ErrorCode::EACCES)));
        assert!(matches!(handles.dup(handle, Rights::NONE), Err(ErrorCode::EPERM)));
        assert!(matches!(BlockDeviceCapability::acquire(0, false), Err(ErrorCode::EBUSY)));
        handles.close(handle).unwrap_or_else(|_| panic!("handle close failed"));
        assert!(BlockDeviceCapability::acquire(0, false).is_ok());
    }
}
