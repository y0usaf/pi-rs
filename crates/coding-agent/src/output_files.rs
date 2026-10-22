//! `utils/output-files.ts`: files holding output the model was not shown in full, such as the
//! whole output of a truncated bash command. They go to the OS temp directory, readable only by
//! the user.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    if let Ok(mut f) = File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut buf);
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// `os.tmpdir()`: `$TMPDIR` without a trailing slash, else `/tmp`.
pub fn tmp_dir() -> String {
    match std::env::var("TMPDIR") {
        Ok(dir) if !dir.is_empty() => {
            let trimmed = dir.trim_end_matches('/');
            if trimmed.is_empty() {
                "/".to_owned()
            } else {
                trimmed.to_owned()
            }
        }
        _ => "/tmp".to_owned(),
    }
}

/// A new, unused path: `<tmp>/<prefix>-<16 hex digits><extension>`.
pub fn new_path(prefix: &str, extension: &str) -> String {
    format!("{}/{prefix}-{}{extension}", tmp_dir(), random_hex(8))
}

/// Creates the file, failing if it exists (`wx`), mode 0600.
pub fn create(path: &str) -> std::io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}
