use alloc::string::String;
use alloc::vec::Vec;
use crate::sync::IrqSafeSpinlock;
use lazy_static::lazy_static;

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
    pub root_fs: Option<&'static dyn FileSystem>,
}

lazy_static! {
    pub static ref VFS: IrqSafeSpinlock<Vfs<'static>> = IrqSafeSpinlock::new(Vfs {
        ramfs_files: Vec::new(),
        root_fs: None,
    });
}

impl<'a> Vfs<'a> {
    pub fn add_file(&mut self, path: String, content: &'a [u8]) {
        self.ramfs_files.push(VirtualFile { path, content });
    }

    pub fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        // Fallback to ramfs if not found in root_fs
        if let Some(fs) = &self.root_fs {
            if let Some(content) = fs.read_file(path) {
                return Some(content);
            }
        }
        self.ramfs_files.iter().find(|f| f.path == path).map(|f| f.content.to_vec())
    }

    pub fn get_files(&self) -> &Vec<VirtualFile<'a>> {
        &self.ramfs_files
    }
}

pub fn init() {
    crate::kprintln!("[vfs] Sistema de archivos virtual inicializado.");
}