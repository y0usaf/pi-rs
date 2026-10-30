use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

pub fn open_browser(target: &str) {
    let mut process = if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        Command::new("xdg-open")
    };
    process
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        process.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    if let Ok(mut child) = process.spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}
