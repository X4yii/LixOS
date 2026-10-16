#![no_std]
#![no_main]
#![allow(unsafe_code)]
extern crate alloc;
use alloc::format;
use authd::store::{AuthStore, BlobStore, StoreError};
use sys_libc::allocator::UserAllocator;
use sys_libc::syscalls::{
    sys_exit, sys_mkdir, sys_read_file_all, sys_remove_file, sys_sleep, sys_write, sys_write_file,
    syscall2,
};
#[global_allocator]
static ALLOCATOR: UserAllocator = UserAllocator::new();
const STORE_DIR: &str = "/var/authd";
const STORE_PATH: &str = "/var/authd/store.bin";
fn klog(text: &str) {
    let bytes = text.as_bytes();
    unsafe {
        syscall2(
            lix_sys::SYS_LOG_WRITE as u64,
            bytes.as_ptr() as u64,
            bytes.len() as u64,
        );
    }
}
fn say(line: &str) {
    sys_write(line);
    sys_write("\n");
    klog(line);
}
struct SyscallBlobStore;
impl BlobStore for SyscallBlobStore {
    fn read(&mut self, path: &str) -> Result<Option<alloc::vec::Vec<u8>>, StoreError> {
        match sys_read_file_all(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(code) if code == -(lix_sys::ErrorCode::ENOENT as i32) => Ok(None),
            Err(_) => Err(StoreError::Io("read failed")),
        }
    }
    fn write_atomic(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let temp = format!("{path}.tmp");
        sys_write_file(&temp, bytes).map_err(|_| StoreError::Io("stage failed"))?;
        if let Err(error) = sys_write_file(path, bytes) {
            let cleanup = sys_remove_file(&temp, false);
            if cleanup.is_err() {
                return Err(StoreError::Io("cleanup failed"));
            }
            return Err(match error {
                _ if error == -(lix_sys::ErrorCode::EACCES as i32) => StoreError::Io("denied"),
                _ => StoreError::Io("commit failed"),
            });
        }
        sys_remove_file(&temp, false).map_err(|_| StoreError::Io("cleanup failed"))
    }
}
fn ensure_store_dir() {
    match sys_mkdir(STORE_DIR) {
        Ok(()) => {}
        Err(code) if code == -(lix_sys::ErrorCode::EEXIST as i32) => {}
        Err(code) => say(&format!("authd: no se pudo crear {STORE_DIR}: {code}")),
    }
}
const fn describe(error: StoreError) -> &'static str {
    match error {
        StoreError::NotFound => "almacén ausente",
        StoreError::HeaderCorrupt | StoreError::Truncated => "cabecera dañada",
        StoreError::ChecksumMismatch { .. } => "checksum de registro inválido",
        StoreError::UnsupportedVersion { .. } => "versión de formato no soportada",
        StoreError::MigrationRequired { .. } => "migración pendiente",
        StoreError::InvalidPayload { .. } => "payload inválido",
        StoreError::MissingRecord(_) | StoreError::DuplicateRecord(_) => "registro faltante",
        StoreError::RecordTooLarge | StoreError::TooManyRecords => "registro fuera de límite",
        StoreError::AtomicWriteUnavailable | StoreError::Io(_) => "error de almacenamiento",
    }
}
fn boot() -> ! {
    say("authd: arrancando");
    let mut io = SyscallBlobStore;
    ensure_store_dir();
    let store = match AuthStore::load(&mut io, STORE_PATH) {
        Ok(store) => store,
        Err(StoreError::NotFound) => {
            let fresh = AuthStore::empty();
            match fresh.save(&mut io, STORE_PATH) {
                Ok(()) => say("authd: almacén creado sin cuentas; alta local pendiente"),
                Err(error) => {
                    say(&format!(
                        "authd: no se pudo crear el almacén: {}",
                        describe(error)
                    ));
                    sys_exit(2);
                }
            }
            fresh
        }
        Err(error) => {
            say(&format!(
                "authd: almacén inválido ({}); se niega a operar",
                describe(error)
            ));
            sys_exit(2);
        }
    };
    say(&format!(
        "authd: almacén válido con {} cuenta(s), intentos máximos {}",
        store.accounts.len(),
        store.policy.max_failed_attempts
    ));
    if store.accounts.is_empty() {
        say("authd: sin credenciales; login y elevación permanecen cerrados");
    }
    let (server_chan, client_chan) = match sys_libc::syscalls::sys_channel_create() {
        Ok(pair) => pair,
        Err(e) => {
            say(&format!("authd: no se pudo crear canal IPC: {e}"));
            loop {
                sys_sleep(1000);
            }
        }
    };
    let _ = sys_write_file("/var/authd/client.handle", &client_chan.0.to_le_bytes());
    say("authd: canal IPC activo escuchando peticiones");
    let mut buf = alloc::vec![0u8; 4096];
    let mut handles = alloc::vec::Vec::new();
    loop {
        match sys_libc::syscalls::sys_channel_recv(server_chan, &mut buf, &mut handles) {
            Ok(n) if n > 0 => {
                let reply = authd::protocol::encode_message(
                    authd::protocol::kind::HELLO_RESPONSE,
                    b"authd ok",
                );
                if let Ok(reply_bytes) = reply {
                    let _ = sys_libc::syscalls::sys_channel_send(server_chan, &reply_bytes, &[]);
                }
                handles.clear();
            }
            _ => {
                sys_sleep(50);
            }
        }
    }
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    boot()
}
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    klog("authd: panic; terminando");
    sys_exit(111);
}
