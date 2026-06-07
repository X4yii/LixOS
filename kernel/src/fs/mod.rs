pub mod tar;
pub mod vfs;
pub mod ext4;

pub fn init() {
    vfs::init();
}
