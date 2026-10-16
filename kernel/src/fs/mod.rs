pub mod error;
pub mod ext4;
pub mod permissions;
pub mod tar;
pub mod vfs;
pub mod journal;
pub fn init() {
    vfs::init();
}
