use core::fmt;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    PermissionDenied,
    InvalidPath,
    AlreadyExists,
    NotEmpty,
    ReadOnly,
    NoSpace,
    OutOfMemory,
    Corrupt,
    Io,
}
impl FsError {
    pub const fn code(self) -> lix_sys::ErrorCode {
        match self {
            FsError::NotFound => lix_sys::ErrorCode::ENOENT,
            FsError::PermissionDenied => lix_sys::ErrorCode::EACCES,
            FsError::InvalidPath => lix_sys::ErrorCode::EINVAL,
            FsError::AlreadyExists => lix_sys::ErrorCode::EEXIST,
            FsError::NotEmpty => lix_sys::ErrorCode::EBUSY,
            FsError::ReadOnly => lix_sys::ErrorCode::EACCES,
            FsError::NoSpace => lix_sys::ErrorCode::ENOMEM,
            FsError::OutOfMemory => lix_sys::ErrorCode::ENOMEM,
            FsError::Corrupt => lix_sys::ErrorCode::EIO,
            FsError::Io => lix_sys::ErrorCode::EIO,
        }
    }
}
impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            FsError::NotFound => "archivo o directorio no encontrado",
            FsError::PermissionDenied => "permiso denegado",
            FsError::InvalidPath => "ruta invalida",
            FsError::AlreadyExists => "el archivo ya existe",
            FsError::NotEmpty => "el directorio no esta vacio",
            FsError::ReadOnly => "sistema de archivos de solo lectura",
            FsError::NoSpace => "sin espacio disponible",
            FsError::OutOfMemory => "sin memoria disponible",
            FsError::Corrupt => "metadatos del sistema de archivos corruptos",
            FsError::Io => "error de entrada/salida",
        };
        f.write_str(message)
    }
}
