#![allow(unsafe_code)]
use std::env;
use std::path::Path;
use std::process::exit;
mod assembler;
mod bench;
mod build;
mod codegen;
mod deps;
mod help;
mod image;
mod installer_test;
mod iso;
mod nvme_test;
mod qemu;
mod smoke;
mod util;
fn main() {
    let mut args = env::args().skip(1);
    let task = args.next().unwrap_or_else(|| "help".to_string());
    let result = match task.as_str() {
        "build" => build::task_build(),
        "check" => build::task_check(),
        "iso" => iso::task_iso(),
        "run" => qemu::task_run(),
        "asm" => assembler::assemble_kernel_asm().map(|_| ()),
        "deps" => deps::ensure_dependencies(),
        "test-nvme" => nvme_test::run(),
        "test-installer" => installer_test::run(),
        "smoke" => smoke::run(),
        "bench" | "microbench" => {
            bench::run_benchmarks();
            Ok(())
        }
        "codegen" => {
            codegen::run();
            Ok(())
        }
        "nvme-image" => {
            let output = env::args()
                .nth(2)
                .unwrap_or_else(|| image::NVME_IMAGE_512.to_string());
            image::create_nvme_test_image(Path::new(&output), 512)
        }
        "nvme-image-4k" => {
            let output = env::args()
                .nth(2)
                .unwrap_or_else(|| image::NVME_IMAGE_4K.to_string());
            image::create_nvme_test_image(Path::new(&output), 4096)
        }
        _ => {
            help::print_help();
            Ok(())
        }
    };
    if let Err(error) = result {
        eprintln!("[xtask] {error}");
        exit(1);
    }
}
