use super::error::FsError;
use crate::sync::IrqSafeSpinlock;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use lazy_static::lazy_static;
const OVERLAY_MAX_BYTES: usize = 32 * 1024 * 1024;
const OVERLAY_MAX_ENTRIES: usize = 1024;
static DISK_READ_OPS: AtomicU64 = AtomicU64::new(0);
static DISK_READ_BYTES: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Copy, Debug, Default)]
pub struct DiskIoStats {
    pub read_ops: u64,
    pub read_bytes: u64,
    pub mounted: bool,
}
pub fn record_disk_read(bytes: usize) {
    DISK_READ_OPS.fetch_add(1, Ordering::Relaxed);
    DISK_READ_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    crate::task::scheduler::with_current_process(|current| {
        current.disk_read_bytes = current.disk_read_bytes.saturating_add(bytes as u64);
    });
}
pub fn disk_io_stats() -> DiskIoStats {
    DiskIoStats {
        read_ops: DISK_READ_OPS.load(Ordering::Relaxed),
        read_bytes: DISK_READ_BYTES.load(Ordering::Relaxed),
        mounted: VFS.lock().root_fs.is_some(),
    }
}
pub trait FileSystem: Send + Sync {
    fn read_file(&self, path: &str) -> Option<Vec<u8>>;
    fn list_files(&self, path: &str) -> Option<Vec<String>>;
    fn write_file(&self, _path: &str, _content: &[u8]) -> Result<(), FsError> {
        Err(FsError::ReadOnly)
    }
}
pub struct VirtualFile<'a> {
    pub path: String,
    pub content: &'a [u8],
}
pub struct Vfs<'a> {
    ramfs_files: Vec<VirtualFile<'a>>,
    overlay_files: Vec<(String, Vec<u8>)>,
    overlay_tombstones: Vec<String>,
    overlay_bytes: usize,
    directories: Vec<String>,
    pub root_fs: Option<&'static dyn FileSystem>,
}
lazy_static! {
    pub static ref VFS: IrqSafeSpinlock<Vfs<'static>> = IrqSafeSpinlock::new(Vfs {
        ramfs_files: Vec::new(),
        overlay_files: Vec::new(),
        overlay_tombstones: Vec::new(),
        overlay_bytes: 0,
        directories: Vec::new(),
        root_fs: None,
    });
}
impl<'a> Vfs<'a> {
    pub fn add_file(&mut self, path: String, content: &'a [u8]) {
        self.ramfs_files.push(VirtualFile { path, content });
    }
    pub fn write_file(&mut self, path: &str, content: &[u8]) -> Result<(), FsError> {
        let path = normalize_path(path);
        if path == "/" {
            return Err(FsError::PermissionDenied);
        }
        if let Some(root_fs) = self.root_fs {
            if root_fs.write_file(&path, content).is_ok() {
                self.overlay_tombstones.retain(|hidden| hidden != &path);
                self.overlay_files.retain(|(name, _)| name != &path);
                return Ok(());
            }
        }
        let Some(new_total) = self
            .overlay_bytes
            .checked_sub(
                self.overlay_files
                    .iter()
                    .find(|(name, _)| name == &path)
                    .map_or(0, |(_, bytes)| bytes.len()),
            )
            .and_then(|bytes| bytes.checked_add(content.len()))
        else {
            return Err(FsError::NoSpace);
        };
        if new_total > OVERLAY_MAX_BYTES {
            return Err(FsError::NoSpace);
        }
        if let Some((_, current)) = self
            .overlay_files
            .iter_mut()
            .find(|(name, _)| name == &path)
        {
            current
                .try_reserve(content.len().saturating_sub(current.len()))
                .map_err(|_| FsError::OutOfMemory)?;
            self.overlay_tombstones.retain(|hidden| hidden != &path);
            current.clear();
            current.extend_from_slice(content);
        } else {
            if self.overlay_files.len() >= OVERLAY_MAX_ENTRIES {
                return Err(FsError::NoSpace);
            }
            self.overlay_files
                .try_reserve(1)
                .map_err(|_| FsError::OutOfMemory)?;
            let mut owned = Vec::new();
            owned
                .try_reserve_exact(content.len())
                .map_err(|_| FsError::OutOfMemory)?;
            owned.extend_from_slice(content);
            self.overlay_tombstones.retain(|hidden| hidden != &path);
            self.overlay_files.push((path, owned));
        }
        self.overlay_bytes = new_total;
        Ok(())
    }
    pub fn mkdir(&mut self, path: &str) -> Result<(), FsError> {
        let path = normalize_path(path);
        if path == "/" {
            return Ok(());
        }
        if self.directories.iter().any(|d| d == &path) {
            return Ok(());
        }
        self.overlay_tombstones.retain(|hidden| hidden != &path);
        self.directories.push(path);
        Ok(())
    }
    pub fn remove_path(&mut self, path: &str, recursive: bool) -> Result<(), FsError> {
        let path = normalize_path(path);
        if path == "/" {
            return Err(FsError::PermissionDenied);
        }
        let prefix = alloc::format!("{}/", path);
        let is_dir = self.directories.iter().any(|d| d == &path)
            || self
                .ramfs_files
                .iter()
                .any(|f| normalize_path(&f.path).starts_with(&prefix))
            || self
                .overlay_files
                .iter()
                .any(|(f, _)| f.starts_with(&prefix));
        if is_dir && !recursive {
            let has_children = self
                .directories
                .iter()
                .any(|d| d != &path && d.starts_with(&prefix))
                || self.ramfs_files.iter().any(|f| {
                    let norm = normalize_path(&f.path);
                    norm.starts_with(&prefix) && !self.overlay_tombstones.iter().any(|t| t == &norm)
                })
                || self
                    .overlay_files
                    .iter()
                    .any(|(f, _)| f.starts_with(&prefix));
            if has_children {
                return Err(FsError::NotEmpty);
            }
        }
        if recursive {
            let to_remove: Vec<String> = self
                .overlay_files
                .iter()
                .filter(|(name, _)| name == &path || name.starts_with(&prefix))
                .map(|(name, _)| name.clone())
                .collect();
            for file_path in to_remove {
                if let Some(idx) = self
                    .overlay_files
                    .iter()
                    .position(|(name, _)| name == &file_path)
                {
                    self.overlay_bytes = self
                        .overlay_bytes
                        .saturating_sub(self.overlay_files[idx].1.len());
                    self.overlay_files.remove(idx);
                }
            }
            self.directories
                .retain(|d| d != &path && !d.starts_with(&prefix));
            if !self.overlay_tombstones.contains(&path) {
                self.overlay_tombstones.push(path.clone());
            }
            let ramfs_matching: Vec<String> = self
                .ramfs_files
                .iter()
                .map(|f| normalize_path(&f.path))
                .filter(|p| p == &path || p.starts_with(&prefix))
                .collect();
            for r in ramfs_matching {
                if !self.overlay_tombstones.contains(&r) {
                    self.overlay_tombstones.push(r);
                }
            }
        } else {
            if let Some(index) = self
                .overlay_files
                .iter()
                .position(|(name, _)| name == &path)
            {
                self.overlay_bytes = self
                    .overlay_bytes
                    .saturating_sub(self.overlay_files[index].1.len());
                self.overlay_files.remove(index);
            }
            self.directories.retain(|d| d != &path);
            if !self.overlay_tombstones.contains(&path) {
                self.overlay_tombstones.push(path);
            }
        }
        Ok(())
    }
}
pub fn canonical(path: &str) -> String {
    normalize_path(path)
}
fn normalize_path(path: &str) -> String {
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            value => components.push(value),
        }
    }
    if components.is_empty() {
        String::from("/")
    } else {
        let mut normalized = String::new();
        for component in components {
            normalized.push('/');
            normalized.push_str(component);
        }
        normalized
    }
}
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    let normalized = normalize_path(path);
    let (root_fs, overlay_content, is_hidden, ramfs_content) = {
        let vfs = VFS.lock();
        let overlay = vfs
            .overlay_files
            .iter()
            .find(|(file_path, _)| file_path == &normalized)
            .map(|(_, content)| content.clone());
        let hidden = vfs
            .overlay_tombstones
            .iter()
            .any(|path| path == &normalized);
        let content = vfs
            .ramfs_files
            .iter()
            .find(|file| normalize_path(&file.path) == normalized)
            .map(|file| file.content);
        (vfs.root_fs, overlay, hidden, content)
    };
    if let Some(content) = overlay_content {
        return Some(content);
    }
    if is_hidden {
        return None;
    }
    if let Some(content) = root_fs.and_then(|fs| fs.read_file(&normalized)) {
        return Some(content);
    }
    ramfs_content.map(|content| content.to_vec())
}
pub fn list_files(path: &str) -> Vec<String> {
    let normalized = normalize_path(path);
    let (root_fs, ramfs_paths, overlay_paths, dir_paths, tombstones) = {
        let vfs = VFS.lock();
        let paths = vfs
            .ramfs_files
            .iter()
            .map(|file| normalize_path(&file.path))
            .collect::<Vec<_>>();
        let overlay = vfs
            .overlay_files
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        let dirs = vfs.directories.clone();
        (
            vfs.root_fs,
            paths,
            overlay,
            dirs,
            vfs.overlay_tombstones.clone(),
        )
    };
    let mut result = root_fs
        .and_then(|fs| fs.list_files(&normalized))
        .unwrap_or_default();
    let prefix = if normalized == "/" {
        String::from("/")
    } else {
        alloc::format!("{}/", normalized)
    };
    for file_path in ramfs_paths
        .into_iter()
        .chain(overlay_paths)
        .chain(dir_paths)
    {
        let Some(relative) = file_path.strip_prefix(&prefix) else {
            continue;
        };
        let Some(child) = relative.split('/').next().filter(|child| !child.is_empty()) else {
            continue;
        };
        let child_path = if normalized == "/" {
            alloc::format!("/{}", child)
        } else {
            alloc::format!("{}/{}", normalized, child)
        };
        if tombstones.iter().any(|hidden| hidden == &child_path) {
            continue;
        }
        if !result.iter().any(|existing| existing == child) {
            result.push(String::from(child));
        }
    }
    result
}
pub fn write_file(path: &str, content: &[u8]) -> Result<(), FsError> {
    VFS.lock().write_file(path, content)
}
pub fn mkdir(path: &str) -> Result<(), FsError> {
    VFS.lock().mkdir(path)
}
pub fn remove_file(path: &str, recursive: bool) -> Result<(), FsError> {
    VFS.lock().remove_path(path, recursive)
}
pub fn init() {
    crate::kprintln!("[vfs] Sistema de archivos virtual inicializado.");
}
