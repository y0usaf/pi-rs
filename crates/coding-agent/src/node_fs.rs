//! Node's `fs` error messages (`ENOENT: no such file or directory, open '/a/b'`), which the tools
//! pass through to the model.

use std::io;

/// The errno name Node reports, like `ENOENT`.
pub fn code(error: &io::Error) -> &'static str {
    match error.raw_os_error() {
        Some(1) => "EPERM",
        Some(2) => "ENOENT",
        Some(5) => "EIO",
        Some(13) => "EACCES",
        Some(17) => "EEXIST",
        Some(20) => "ENOTDIR",
        Some(21) => "EISDIR",
        Some(22) => "EINVAL",
        Some(24) => "EMFILE",
        Some(28) => "ENOSPC",
        Some(30) => "EROFS",
        Some(36) => "ENAMETOOLONG",
        Some(39) => "ENOTEMPTY",
        Some(40) => "ELOOP",
        _ => match error.kind() {
            io::ErrorKind::NotFound => "ENOENT",
            io::ErrorKind::PermissionDenied => "EACCES",
            io::ErrorKind::AlreadyExists => "EEXIST",
            io::ErrorKind::IsADirectory => "EISDIR",
            io::ErrorKind::NotADirectory => "ENOTDIR",
            _ => "EIO",
        },
    }
}

fn description(code: &str) -> &'static str {
    match code {
        "EPERM" => "operation not permitted",
        "ENOENT" => "no such file or directory",
        "EACCES" => "permission denied",
        "EEXIST" => "file already exists",
        "ENOTDIR" => "not a directory",
        "EISDIR" => "illegal operation on a directory",
        "EINVAL" => "invalid argument",
        "EMFILE" => "too many open files",
        "ENOSPC" => "no space left on device",
        "EROFS" => "read-only file system",
        "ENAMETOOLONG" => "name too long",
        "ENOTEMPTY" => "directory not empty",
        "ELOOP" => "too many symbolic links encountered",
        _ => "i/o error",
    }
}

/// `CODE: description, syscall 'path'`, the message of a Node fs error.
pub fn message(error: &io::Error, syscall: &str, path: &str) -> String {
    let code = code(error);
    format!("{code}: {}, {syscall} '{path}'", description(code))
}

/// `access(path, mode)`: Node checks permissions with the real uid; `R_OK` is 4, `W_OK` 2.
pub fn access(path: &str, mode: i32) -> io::Result<()> {
    let c = std::ffi::CString::new(path).map_err(|_| io::Error::from_raw_os_error(2))?;
    unsafe extern "C" {
        fn access(path: *const std::ffi::c_char, mode: i32) -> i32;
    }
    // SAFETY: `c` is a valid NUL-terminated string for the duration of the call.
    if unsafe { access(c.as_ptr(), mode) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub const R_OK: i32 = 4;
pub const W_OK: i32 = 2;
