use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;
pub fn run_checked(program: &str, args: &[&str], context: &str) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|error| format!("{context}: no se pudo ejecutar {program}: {error}"))?;
    if !status.success() {
        return Err(format!("{context}: {program} terminó con estado {status}"));
    }
    Ok(())
}
pub fn ensure_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("no se pudo crear {}: {error}", path.display()))
}
pub fn copy_required(source: &Path, destination: &Path) -> Result<(), String> {
    fs::copy(source, destination).map(|_| ()).map_err(|error| {
        format!(
            "no se pudo copiar {} a {}: {error}",
            source.display(),
            destination.display()
        )
    })
}
pub fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("no se pudo eliminar {}: {error}", path.display())),
    }
}
pub fn write_required(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents)
        .map_err(|error| format!("no se pudo escribir {}: {error}", path.display()))
}
pub fn write_if_missing(path: &Path, contents: &str) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    write_required(path, contents)
}
