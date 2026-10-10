use alloc::string::String;
use alloc::vec::Vec;
use crate::sync::IrqSafeSpinlock;
use lazy_static::lazy_static;
use core::sync::atomic::{AtomicU64, Ordering};

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

/// Record successful physical reads performed by the mounted VFS filesystem.
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
}

pub struct VirtualFile<'a> {
    pub path: String,
    pub content: &'a [u8],
}

pub struct Vfs<'a> {
    ramfs_files: Vec<VirtualFile<'a>>,
    /// Writable upper layer; the mounted root filesystem remains read-only.
    overlay_files: Vec<(String, Vec<u8>)>,
    overlay_tombstones: Vec<String>,
    overlay_bytes: usize,
    pub root_fs: Option<&'static dyn FileSystem>,
}

lazy_static! {
    pub static ref VFS: IrqSafeSpinlock<Vfs<'static>> = IrqSafeSpinlock::new(Vfs {
        ramfs_files: Vec::new(),
        overlay_files: Vec::new(),
        overlay_tombstones: Vec::new(),
        overlay_bytes: 0,
        root_fs: None,
    });
}

impl<'a> Vfs<'a> {
    pub fn add_file(&mut self, path: String, content: &'a [u8]) {
        self.ramfs_files.push(VirtualFile { path, content });
    }

    /// Creates or replaces a file in the in-memory upper layer (never writes the lower FS).
    pub fn write_file(&mut self, path: &str, content: &[u8]) -> Result<(), &'static str> {
        let path = normalize_path(path);
        if path == "/" { return Err("cannot replace the VFS root with a file"); }
        let Some(new_total) = self.overlay_bytes
            .checked_sub(self.overlay_files.iter().find(|(name, _)| name == &path).map_or(0, |(_, bytes)| bytes.len()))
            .and_then(|bytes| bytes.checked_add(content.len())) else { return Err("overlay size overflow"); };
        if new_total > OVERLAY_MAX_BYTES { return Err("tmpfs overlay capacity exceeded"); }
        if let Some((_, current)) = self.overlay_files.iter_mut().find(|(name, _)| name == &path) {
            current.try_reserve(content.len().saturating_sub(current.len())).map_err(|_| "out of memory")?;
            self.overlay_tombstones.retain(|hidden| hidden != &path);
            current.clear();
            current.extend_from_slice(content);
        } else {
            if self.overlay_files.len() >= OVERLAY_MAX_ENTRIES { return Err("tmpfs overlay entry limit exceeded"); }
            self.overlay_files.try_reserve(1).map_err(|_| "out of memory")?;
            let mut owned = Vec::new();
            owned.try_reserve_exact(content.len()).map_err(|_| "out of memory")?;
            owned.extend_from_slice(content);
            self.overlay_tombstones.retain(|hidden| hidden != &path);
            self.overlay_files.push((path, owned));
        }
        self.overlay_bytes = new_total;
        Ok(())
    }

    /// Hides a file from all lower layers without changing the read-only root.
    pub fn remove_file(&mut self, path: &str) -> Result<(), &'static str> {
        let path = normalize_path(path);
        if !self.overlay_tombstones.iter().any(|hidden| hidden == &path)
            && self.overlay_tombstones.len() >= OVERLAY_MAX_ENTRIES
        { return Err("tmpfs tombstone limit exceeded"); }
        let add_tombstone = !self.overlay_tombstones.iter().any(|hidden| hidden == &path);
        if add_tombstone { self.overlay_tombstones.try_reserve(1).map_err(|_| "out of memory")?; }
        if let Some(index) = self.overlay_files.iter().position(|(name, _)| name == &path) {
            self.overlay_bytes = self.overlay_bytes.saturating_sub(self.overlay_files[index].1.len());
        }
        self.overlay_files.retain(|(name, _)| name != &path);
        if add_tombstone { self.overlay_tombstones.push(path); }
        Ok(())
    }

}

fn normalize_path(path: &str) -> String {
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => { components.pop(); }
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

/// Resolve outside the VFS lock so filesystem drivers may perform blocking I/O.
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    let normalized = normalize_path(path);
    let (root_fs, overlay_content, is_hidden, ramfs_content) = {
        let vfs = VFS.lock();
        let overlay = vfs.overlay_files.iter()
            .find(|(file_path, _)| file_path == &normalized)
            .map(|(_, content)| content.clone());
        let hidden = vfs.overlay_tombstones.iter().any(|path| path == &normalized);
        let content = vfs.ramfs_files.iter()
            .find(|file| normalize_path(&file.path) == normalized)
            .map(|file| file.content);
        (vfs.root_fs, overlay, hidden, content)
    };

    if let Some(content) = overlay_content { return Some(content); }
    if is_hidden { return None; }
    if let Some(content) = root_fs.and_then(|fs| fs.read_file(&normalized)) {
        return Some(content);
    }
    ramfs_content.map(|content| content.to_vec())
}

/// Lists direct children from the mounted filesystem and initrd without holding the VFS lock
/// during filesystem I/O.
pub fn list_files(path: &str) -> Vec<String> {
    let normalized = normalize_path(path);
    let (root_fs, ramfs_paths, overlay_paths, tombstones) = {
        let vfs = VFS.lock();
        let paths = vfs.ramfs_files.iter()
            .map(|file| normalize_path(&file.path))
            .collect::<Vec<_>>();
        let overlay = vfs.overlay_files.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>();
        (vfs.root_fs, paths, overlay, vfs.overlay_tombstones.clone())
    };

    let mut result = root_fs.and_then(|fs| fs.list_files(&normalized)).unwrap_or_default();
    let prefix = if normalized == "/" {
        String::from("/")
    } else {
        alloc::format!("{}/", normalized)
    };
    for file_path in ramfs_paths.into_iter().chain(overlay_paths) {
        let Some(relative) = file_path.strip_prefix(&prefix) else { continue; };
        let Some(child) = relative.split('/').next().filter(|child| !child.is_empty()) else { continue; };
        let child_path = if normalized == "/" { alloc::format!("/{}", child) } else { alloc::format!("{}/{}", normalized, child) };
        if tombstones.iter().any(|hidden| hidden == &child_path) { continue; }
        if !result.iter().any(|existing| existing == child) {
            result.push(String::from(child));
        }
    }
    result
}

pub fn init() {
    crate::kprintln!("[vfs] Sistema de archivos virtual inicializado.");
}
