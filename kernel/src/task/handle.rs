use crate::drivers::block::{BlockDevice, BlockDeviceEntry, MAX_BLOCK_DEVICES};
use crate::task::process::ProcessId;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use lix_sys::{ErrorCode, Handle, Rights};
static BLOCK_LEASES: AtomicU64 = AtomicU64::new(0);
fn lease_bit(device_index: u32) -> u64 {
    1u64 << device_index
}
fn try_lease(device_index: u32) -> Result<(), ErrorCode> {
    if device_index as usize >= MAX_BLOCK_DEVICES {
        return Err(ErrorCode::ENODEV);
    }
    let bit = lease_bit(device_index);
    let mut current = BLOCK_LEASES.load(Ordering::Relaxed);
    loop {
        if current & bit != 0 {
            return Err(ErrorCode::EBUSY);
        }
        match BLOCK_LEASES.compare_exchange_weak(
            current,
            current | bit,
            Ordering::Acquire,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Ok(()),
            Err(actual) => current = actual,
        }
    }
}
fn release_lease(device_index: u32) {
    BLOCK_LEASES.fetch_and(!lease_bit(device_index), Ordering::Release);
}
pub struct BlockDeviceCapability {
    pub device_index: u32,
    pub name: &'static str,
    pub device: &'static dyn BlockDevice,
    pub read_only: bool,
    pub writable: bool,
}
impl core::fmt::Debug for BlockDeviceCapability {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BlockDeviceCapability")
            .field("device_index", &self.device_index)
            .field("name", &self.name)
            .field("read_only", &self.read_only)
            .field("writable", &self.writable)
            .finish()
    }
}
impl BlockDeviceCapability {
    pub fn acquire(
        device_index: u32,
        entry: BlockDeviceEntry,
        writable: bool,
    ) -> Result<Arc<Self>, ErrorCode> {
        if writable && entry.read_only {
            return Err(ErrorCode::EACCES);
        }
        try_lease(device_index)?;
        Ok(Arc::new(Self {
            device_index,
            name: entry.name,
            device: entry.device,
            read_only: entry.read_only,
            writable,
        }))
    }
}
impl Drop for BlockDeviceCapability {
    fn drop(&mut self) {
        release_lease(self.device_index);
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
    InputDevice {
        class: u32,
    },
    UdpSocket(Arc<crate::drivers::net::UdpSocket>),
    Channel(Arc<crate::task::channel::ChannelEndpoint>),
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
    pub fn get_mut(
        &mut self,
        handle: Handle,
        required_rights: Rights,
    ) -> Result<&mut KernelObject, ErrorCode> {
        let idx = handle.index() as usize;
        let gen = handle.generation();
        if idx >= self.slots.len() {
            return Err(ErrorCode::EBADF);
        }
        let is_fixed_stdio = gen == 0 && idx < 3;
        match &mut self.slots[idx] {
            Some(entry) => {
                if !is_fixed_stdio && entry.generation != gen {
                    return Err(ErrorCode::EBADF);
                }
                if !entry.rights.contains(required_rights) {
                    return Err(ErrorCode::EACCES);
                }
                Ok(&mut entry.object)
            }
            None => Err(ErrorCode::EBADF),
        }
    }
    pub fn has_right(&self, required: Rights) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|entry| entry.rights.contains(required))
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
    pub fn take(
        &mut self,
        handle: Handle,
        required_rights: Rights,
    ) -> Result<(KernelObject, Rights), ErrorCode> {
        let idx = handle.index() as usize;
        let gen = handle.generation();
        if idx >= self.slots.len() {
            return Err(ErrorCode::EBADF);
        }
        if gen == 0 && idx < 3 {
            return Err(ErrorCode::EPERM);
        }
        match &self.slots[idx] {
            Some(entry) => {
                if entry.generation != gen {
                    return Err(ErrorCode::EBADF);
                }
                if !entry.rights.contains(required_rights) {
                    return Err(ErrorCode::EACCES);
                }
            }
            None => return Err(ErrorCode::EBADF),
        }
        let entry = self.slots[idx].take().unwrap();
        self.generations[idx] = self.generations[idx].wrapping_add(1).max(1);
        Ok((entry.object, entry.rights))
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
    pub fn dup2(&mut self, src: Handle, dst_idx: usize) -> Result<Handle, ErrorCode> {
        if dst_idx >= 4096 {
            return Err(ErrorCode::EMFILE);
        }
        let src_idx = src.index() as usize;
        if src_idx >= self.slots.len() {
            return Err(ErrorCode::EBADF);
        }
        let entry = match &self.slots[src_idx] {
            Some(e) => {
                if src.generation() != 0 && e.generation != src.generation() {
                    return Err(ErrorCode::EBADF);
                }
                if !e.rights.contains(Rights::DUP) {
                    return Err(ErrorCode::EPERM);
                }
                e.clone()
            }
            None => return Err(ErrorCode::EBADF),
        };
        while self.slots.len() <= dst_idx {
            self.slots.push(None);
            self.generations.push(1);
        }
        let gen = self.generations[dst_idx];
        self.slots[dst_idx] = Some(HandleEntry {
            object: entry.object,
            rights: entry.rights,
            generation: gen,
        });
        Ok(Handle::new(dst_idx as u32, gen))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::block::{BlockDevice, BlockError, BlockGeometry, DmaBuffer};
    struct MockDevice;
    impl BlockDevice for MockDevice {
        fn geometry(&self) -> Result<BlockGeometry, BlockError> {
            Ok(BlockGeometry {
                block_count: 512,
                block_size: 512,
            })
        }
        fn read_blocks(&self, _: u64, _: u16, _: &mut DmaBuffer) -> Result<(), BlockError> {
            Ok(())
        }
        fn write_blocks(&self, _: u64, _: u16, _: &DmaBuffer) -> Result<(), BlockError> {
            Ok(())
        }
    }
    static MOCK: MockDevice = MockDevice;
    fn entry(read_only: bool) -> BlockDeviceEntry {
        BlockDeviceEntry {
            name: "mock",
            device: &MOCK,
            read_only,
        }
    }
    #[test]
    fn device_lease_is_exclusive_and_released_on_drop() {
        let first = match BlockDeviceCapability::acquire(0, entry(false), false) {
            Ok(capability) => capability,
            Err(_) => panic!("first lease was unexpectedly unavailable"),
        };
        assert!(matches!(
            BlockDeviceCapability::acquire(0, entry(false), true),
            Err(ErrorCode::EBUSY)
        ));
        drop(first);
        let second = match BlockDeviceCapability::acquire(0, entry(false), true) {
            Ok(capability) => capability,
            Err(_) => panic!("lease was not released on drop"),
        };
        assert!(second.writable);
    }
    #[test]
    fn lease_rejects_out_of_range_device_indices() {
        assert!(matches!(
            BlockDeviceCapability::acquire(200, entry(false), false),
            Err(ErrorCode::ENODEV)
        ));
    }
    #[test]
    fn lease_rejects_writes_to_read_only_devices() {
        assert!(matches!(
            BlockDeviceCapability::acquire(3, entry(true), true),
            Err(ErrorCode::EACCES)
        ));
    }
    #[test]
    fn block_handle_rights_and_close_control_the_lease() {
        let capability = match BlockDeviceCapability::acquire(2, entry(false), false) {
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
        assert!(matches!(
            handles.get(handle, Rights::WRITE),
            Err(ErrorCode::EACCES)
        ));
        assert!(matches!(
            handles.dup(handle, Rights::NONE),
            Err(ErrorCode::EPERM)
        ));
        assert!(matches!(
            BlockDeviceCapability::acquire(2, entry(false), false),
            Err(ErrorCode::EBUSY)
        ));
        handles
            .close(handle)
            .unwrap_or_else(|_| panic!("handle close failed"));
        assert!(BlockDeviceCapability::acquire(2, entry(false), false).is_ok());
    }
}
