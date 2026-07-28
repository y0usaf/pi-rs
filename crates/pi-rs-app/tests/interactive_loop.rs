#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! PTY-driven acceptance test for the interactive product loop.
//!
//! Spawns `pi` behind a real pseudo-terminal, types input, and verifies the
//! loop renders ANSI frames and exits on a shutdown action.

use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Pty {
    master: std::fs::File,
    slave: std::fs::File,
}

fn open_pty() -> Pty {
    unsafe {
        let master_fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        assert!(master_fd >= 0, "posix_openpt failed");
        assert_eq!(libc::grantpt(master_fd), 0, "grantpt failed");
        assert_eq!(libc::unlockpt(master_fd), 0, "unlockpt failed");
        let slave_name = libc::ptsname(master_fd);
        assert!(!slave_name.is_null(), "ptsname failed");
        let slave_fd = libc::open(slave_name, libc::O_RDWR | libc::O_NOCTTY);
        assert!(slave_fd >= 0, "open slave PTY failed");
        Pty {
            master: std::fs::File::from_raw_fd(master_fd),
            slave: std::fs::File::from_raw_fd(slave_fd),
        }
    }
}

fn read_available(master: &mut std::fs::File, timeout: Duration) -> String {
    let mut output = Vec::new();
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let fd = master.as_raw_fd();
        let mut fd_set = unsafe { std::mem::zeroed::<libc::fd_set>() };
        unsafe {
            libc::FD_ZERO(&mut fd_set);
            libc::FD_SET(fd, &mut fd_set);
        }
        let mut tv = libc::timeval {
            tv_sec: remaining.as_secs() as _,
            tv_usec: remaining.subsec_micros() as _,
        };
        let ready = unsafe {
            libc::select(
                fd + 1,
                &mut fd_set,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut tv,
            )
        };
        if ready <= 0 {
            break;
        }
        let mut buf = [0_u8; 4096];
        match master.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => output.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

#[test]
fn pty_loop_renders_input_frames_and_exits_on_shutdown() {
    let scratch = tempfile::tempdir().unwrap();
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/walking-skeleton/application.lua"
        ),
        scratch.path().join("application.lua"),
    )
    .unwrap();
    std::fs::write(
        scratch.path().join("packages.json"),
        r#"{"version":1,"packages":["application.lua"]}"#,
    )
    .unwrap();

    let mut pty = open_pty();
    let slave_raw: RawFd = pty.slave.as_raw_fd();

    // Duplicate the slave fd for the child's stdin/stdout. The parent
    // keeps its own copy; the child inherits duplicated descriptors.
    let child_stdin = unsafe { libc::dup(slave_raw) };
    let child_stdout = unsafe { libc::dup(slave_raw) };
    assert!(child_stdin >= 0 && child_stdout >= 0);

    let child = Command::new(env!("CARGO_BIN_EXE_pi"))
        .current_dir(scratch.path())
        .env("HOME", scratch.path())
        .env("TERM", "xterm-256color")
        .env_remove("PI_PACKAGE_MANIFEST")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CACHE_HOME")
        .args(["--manifest", "packages.json"])
        .stdin(unsafe { Stdio::from_raw_fd(child_stdin) })
        .stdout(unsafe { Stdio::from_raw_fd(child_stdout) })
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pi behind PTY");

    // Wait for the startup frame.
    let startup_output = read_available(&mut pty.master, Duration::from_secs(5));
    assert!(
        !startup_output.is_empty(),
        "startup frame should produce ANSI output"
    );

    // Type 'h' — the skeleton should echo it.
    pty.master.write_all(b"h").unwrap();
    let echo_output = read_available(&mut pty.master, Duration::from_secs(2));
    assert!(
        !echo_output.is_empty(),
        "typed input should produce ANSI output"
    );

    // Type 'q' — the skeleton should shut down.
    pty.master.write_all(b"q").unwrap();

    let output = child.wait_with_output().expect("wait for pi to exit");
    assert!(
        output.status.success(),
        "pi should exit cleanly: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}
