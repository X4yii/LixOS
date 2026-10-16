use crate::task::syscall::{
    copy_from_user, copy_from_user_slice, copy_to_user_slice, validate_user_buffer,
};
use lix_net::Ipv4Address;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SocketAddrV4 {
    pub family: u16,
    pub port_be: u16,
    pub address: [u8; 4],
    pub reserved: [u8; 8],
}
fn errno(error: lix_sys::ErrorCode) -> u64 {
    -(error as i32) as u64
}
fn socket(
    handle: u64,
    rights: lix_sys::Rights,
) -> Result<alloc::sync::Arc<crate::drivers::net::UdpSocket>, lix_sys::ErrorCode> {
    crate::task::scheduler::with_current_process(|p| {
        match p.handles.get(lix_sys::Handle(handle), rights)? {
            crate::task::handle::KernelObject::UdpSocket(s) => Ok(s.clone()),
            _ => Err(lix_sys::ErrorCode::EBADF),
        }
    })
    .unwrap_or(Err(lix_sys::ErrorCode::ESRCH))
}
pub fn sys_socket_open() -> u64 {
    let result = crate::task::scheduler::with_current_process(|p| {
        if !p.rights.contains(lix_sys::Rights::NET_CONNECT)
            && !p.handles.has_right(lix_sys::Rights::NET_CONNECT)
        {
            return Err(lix_sys::ErrorCode::EACCES);
        }
        let sock = crate::drivers::net::UdpSocket::new();
        p.handles.insert(
            crate::task::handle::KernelObject::UdpSocket(sock),
            lix_sys::Rights(
                lix_sys::Rights::READ.0 | lix_sys::Rights::WRITE.0 | lix_sys::Rights::DUP.0,
            ),
        )
    })
    .unwrap_or(Err(lix_sys::ErrorCode::ESRCH));
    result.map(|h| h.0).unwrap_or_else(errno)
}
pub fn sys_socket_bind(handle: u64, port: u64) -> u64 {
    if port == 0 || port > u16::MAX as u64 {
        return errno(lix_sys::ErrorCode::EINVAL);
    }
    let sock = match socket(handle, lix_sys::Rights::WRITE) {
        Ok(s) => s,
        Err(e) => return errno(e),
    };
    match sock.bind(port as u16) {
        Ok(()) => 0,
        Err(crate::drivers::net::NetworkError::PortInUse) => errno(lix_sys::ErrorCode::EADDRINUSE),
        _ => errno(lix_sys::ErrorCode::EINVAL),
    }
}
const MAX_UDP_PAYLOAD: usize = 1472;
pub fn sys_socket_sendto(handle: u64, addr_ptr: u64, data_ptr: u64, length: usize) -> u64 {
    if length > MAX_UDP_PAYLOAD {
        return errno(lix_sys::ErrorCode::EINVAL);
    }
    if validate_user_buffer(addr_ptr, core::mem::size_of::<SocketAddrV4>(), false).is_err()
        || validate_user_buffer(data_ptr, length, false).is_err()
    {
        return errno(lix_sys::ErrorCode::EFAULT);
    }
    let addr = match copy_from_user(addr_ptr as *const SocketAddrV4) {
        Ok(addr) => addr,
        Err(_) => return errno(lix_sys::ErrorCode::EFAULT),
    };
    if addr.family != 2 {
        return errno(lix_sys::ErrorCode::EINVAL);
    }
    let mut data = [0u8; MAX_UDP_PAYLOAD];
    if copy_from_user_slice(data_ptr as *const u8, &mut data[..length]).is_err() {
        return errno(lix_sys::ErrorCode::EFAULT);
    }
    let sock = match socket(handle, lix_sys::Rights::WRITE) {
        Ok(s) => s,
        Err(e) => return errno(e),
    };
    match sock.send_to(Ipv4Address(addr.address), u16::from_be(addr.port_be), &data[..length]) {
        Ok(()) => length as u64,
        Err(crate::drivers::net::NetworkError::NoDevice) => errno(lix_sys::ErrorCode::ENODEV),
        Err(_) => errno(lix_sys::ErrorCode::EAGAIN),
    }
}
pub fn sys_socket_recvfrom(handle: u64, data_ptr: u64, capacity: usize, addr_ptr: u64) -> u64 {
    if capacity > 65536
        || validate_user_buffer(data_ptr, capacity, true).is_err()
        || validate_user_buffer(addr_ptr, core::mem::size_of::<SocketAddrV4>(), true).is_err()
    {
        return errno(lix_sys::ErrorCode::EFAULT);
    }
    let sock = match socket(handle, lix_sys::Rights::READ) {
        Ok(s) => s,
        Err(e) => return errno(e),
    };
    let mut temp = [0u8; MAX_UDP_PAYLOAD];
    let recv_len = core::cmp::min(capacity, MAX_UDP_PAYLOAD);
    match sock.receive(&mut temp[..recv_len]) {
        Ok(Some((source, port, length))) => {
            if copy_to_user_slice(data_ptr as *mut u8, &temp[..length]).is_err() {
                return errno(lix_sys::ErrorCode::EFAULT);
            }
            let addr = SocketAddrV4 {
                family: 2,
                port_be: port.to_be(),
                address: source.0,
                reserved: [0; 8],
            };
            if unsafe { crate::task::syscall::copy_to_user(addr_ptr as *mut SocketAddrV4, &addr) }
                .is_err()
            {
                return errno(lix_sys::ErrorCode::EFAULT);
            }
            length as u64
        }
        Ok(None) => errno(lix_sys::ErrorCode::EAGAIN),
        Err(_) => errno(lix_sys::ErrorCode::EINVAL),
    }
}
