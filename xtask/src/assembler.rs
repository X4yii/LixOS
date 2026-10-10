use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub enum AssemblerTool {
    Clang,
    Gcc,
    As,
}

fn detect_assembler() -> Result<AssemblerTool, String> {
    if Command::new("clang").arg("--version").output().is_ok() {
        return Ok(AssemblerTool::Clang);
    }
    if Command::new("gcc").arg("--version").output().is_ok() {
        return Ok(AssemblerTool::Gcc);
    }
    if Command::new("as").arg("--version").output().is_ok() {
        return Ok(AssemblerTool::As);
    }
    Err("No se encontro un ensamblador compatible en PATH (clang, gcc o as)".into())
}

fn needs_rebuild(source: &Path, object: &Path) -> bool {
    let Ok(src_meta) = fs::metadata(source) else { return true; };
    let Ok(obj_meta) = fs::metadata(object) else { return true; };
    let (Ok(src_time), Ok(obj_time)) = (src_meta.modified(), obj_meta.modified()) else { return true; };
    src_time > obj_time
}

pub fn assemble_file(source: &Path, object: &Path, tool: AssemblerTool) -> Result<(), String> {
    if let Some(parent) = object.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let status = match tool {
        AssemblerTool::Clang => {
            Command::new("clang")
                .args([
                    "-c",
                    "--target=x86_64-unknown-none",
                    "-nostdlib",
                    "-mcmodel=kernel",
                    "-fno-pie",
                    "-fno-stack-protector",
                    "-O2",
                ])
                .arg(source)
                .arg("-o")
                .arg(object)
                .status()
                .map_err(|e| format!("clang: {e}"))?
        }
        AssemblerTool::Gcc => {
            Command::new("gcc")
                .args([
                    "-c",
                    "-m64",
                    "-nostdlib",
                    "-mcmodel=kernel",
                    "-fno-pie",
                    "-fno-stack-protector",
                    "-O2",
                ])
                .arg(source)
                .arg("-o")
                .arg(object)
                .status()
                .map_err(|e| format!("gcc: {e}"))?
        }
        AssemblerTool::As => {
            Command::new("as")
                .args(["--64"])
                .arg(source)
                .arg("-o")
                .arg(object)
                .status()
                .map_err(|e| format!("as: {e}"))?
        }
    };

    if !status.success() {
        return Err(format!("Fallo al ensamblar {}", source.display()));
    }

    Ok(())
}

pub fn assemble_directory(source_dir: &Path, target_dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !source_dir.exists() {
        return Ok(Vec::new());
    }

    let tool = detect_assembler()?;
    let mut objects = Vec::new();

    let entries = fs::read_dir(source_dir).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_file() {
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if ext.eq_ignore_ascii_case("s") || ext.eq_ignore_ascii_case("asm") {
                let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("asm");
                let obj_path = target_dir.join(format!("{file_stem}.o"));

                if needs_rebuild(&path, &obj_path) {
                    println!("[xtask] Ensamblando {} -> {}", path.display(), obj_path.display());
                    assemble_file(&path, &obj_path, tool)?;
                }
                objects.push(obj_path);
            }
        }
    }

    objects.sort();
    Ok(objects)
}

pub fn assemble_kernel_asm() -> Result<Vec<PathBuf>, String> {
    let src_dir = Path::new("kernel/asm");
    let out_dir = Path::new("target/x86_64-unknown-none/asm_objs");
    assemble_directory(src_dir, out_dir)
}
