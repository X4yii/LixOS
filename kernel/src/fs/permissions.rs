use super::vfs;
use lix_sys::{ErrorCode, Rights};
pub const PROTECTED_PREFIXES: &[&str] = &["/bin", "/boot", "/etc", "/lib", "/usr", "/root"];
pub const SECRET_PREFIXES: &[&str] = &["/root", "/var/lib/auth", "/var/authd", "/etc/shadow"];
fn is_prefix_match(path: &str, prefix: &str) -> bool {
    path == prefix || (path.starts_with(prefix) && path[prefix.len()..].starts_with('/'))
}
fn is_protected(path: &str) -> bool {
    PROTECTED_PREFIXES
        .iter()
        .any(|prefix| is_prefix_match(path, prefix))
}
fn is_secret(path: &str) -> bool {
    SECRET_PREFIXES
        .iter()
        .any(|prefix| is_prefix_match(path, prefix))
}
fn shape(path: &str) -> Result<(), ErrorCode> {
    if !path.starts_with('/') {
        return Err(ErrorCode::EINVAL);
    }
    if path.split('/').any(|component| component == "..") {
        return Err(ErrorCode::EACCES);
    }
    Ok(())
}
pub fn authorize_read(path: &str, rights: Rights) -> Result<(), u64> {
    shape(path).map_err(|error| -(error as i32) as u64)?;
    if !rights.contains(Rights::READ) {
        return Err(-(ErrorCode::EACCES as i32) as u64);
    }
    let normalized = vfs::canonical(path);
    if is_secret(&normalized) && !rights.contains(Rights::AUTH) {
        return Err(-(ErrorCode::EACCES as i32) as u64);
    }
    Ok(())
}
pub fn authorize_write(path: &str, rights: Rights) -> Result<(), u64> {
    shape(path).map_err(|error| -(error as i32) as u64)?;
    let normalized = vfs::canonical(path);
    if normalized == "/" {
        return Err(-(ErrorCode::EACCES as i32) as u64);
    }
    if is_protected(&normalized) {
        if !rights.contains(Rights::AUTH) {
            return Err(-(ErrorCode::EACCES as i32) as u64);
        }
        return Ok(());
    }
    if !rights.contains(Rights::WRITE) {
        return Err(-(ErrorCode::EACCES as i32) as u64);
    }
    Ok(())
}
pub fn authorize_remove(path: &str, rights: Rights) -> Result<(), u64> {
    authorize_write(path, rights)
}
