use crate::sync::IrqSafeSpinlock;
use alloc::vec::Vec;
pub use lix_usb::{Descriptor, DescriptorError, DescriptorIter, InterfaceDescriptor};
pub use lix_sys::UsbDeviceInfo;
pub const DEVICE_CONNECTED: u32 = 1;
pub const DEVICE_CONFIGURED: u32 = 2;
static DEVICES: IrqSafeSpinlock<Vec<UsbDeviceInfo>> = IrqSafeSpinlock::new(Vec::new());
pub fn register(info: UsbDeviceInfo) {
    let mut devices = DEVICES.lock();
    if let Some(existing) = devices.iter_mut().find(|device| device.id == info.id) {
        *existing = info;
    } else {
        devices.push(info);
    }
}
pub fn remove(id: u16) {
    DEVICES.lock().retain(|device| device.id != id);
}
pub fn enumerate(out: &mut [UsbDeviceInfo]) -> usize {
    let devices = DEVICES.lock();
    let count = core::cmp::min(out.len(), devices.len());
    out[..count].copy_from_slice(&devices[..count]);
    count
}
pub const fn configuration_descriptors(bytes: &[u8]) -> DescriptorIter<'_> {
    DescriptorIter::new(bytes)
}
