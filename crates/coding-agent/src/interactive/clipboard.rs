use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use pi_tui::osc52;
use pi_tui::utils::js_trim;

const MAX_OSC52_ENCODED_LENGTH: usize = 100_000;
const MAX_BUFFER_BYTES: usize = 50 * 1024 * 1024;

fn set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn is_remote_session() -> bool {
    set("SSH_CONNECTION")
        || set("SSH_CLIENT")
        || set("MOSH_CONNECTION")
        || std::env::var("PI_CLIPBOARD_OSC52").is_ok_and(|value| value == "1")
}

fn is_wsl() -> bool {
    if set("WSL_DISTRO_NAME") || set("WSLENV") {
        return true;
    }
    std::fs::read_to_string("/proc/version").is_ok_and(|release| {
        let release = release.to_lowercase();
        release.contains("microsoft") || release.contains("wsl")
    })
}

fn emit_osc52(text: &str) -> bool {
    let encoded = osc52::base64(text.as_bytes());
    if encoded.len() > MAX_OSC52_ENCODED_LENGTH {
        return false;
    }
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(osc52::sequence(&encoded).as_bytes());
    let _ = out.flush();
    true
}

pub fn run_clipboard_command(
    command: &str,
    args: &[&str],
    input: Option<&str>,
    timeout: Duration,
) -> Option<Vec<u8>> {
    let mut process = Command::new(command);
    process
        .args(args)
        .stdin(Stdio::piped())
        .stdout(if input.is_some() {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stderr(Stdio::null());
    unsafe {
        process.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = process.spawn().ok()?;
    let output = collect(
        &mut child,
        input.unwrap_or_default().as_bytes(),
        Instant::now() + timeout,
    );
    if output.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    output
}

fn set_nonblocking(pipe: &impl AsRawFd) {
    let fd = pipe.as_raw_fd();
    unsafe {
        libc::fcntl(
            fd,
            libc::F_SETFL,
            libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK,
        );
    }
}

fn collect(child: &mut Child, input: &[u8], deadline: Instant) -> Option<Vec<u8>> {
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take();
    stdin.iter().for_each(set_nonblocking);
    stdout.iter().for_each(set_nonblocking);
    let mut written = 0;
    let mut output = Vec::new();
    let mut chunk = vec![0; 65536];
    loop {
        stdin = stdin.filter(|_| written < input.len());
        if let Some(pipe) = stdin.as_mut() {
            match pipe.write(&input[written..]) {
                Ok(count) => written += count,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(_) => stdin = None,
            }
        }
        while let Some(pipe) = stdout.as_mut() {
            match pipe.read(&mut chunk) {
                Ok(0) => stdout = None,
                Ok(count) => {
                    output.extend_from_slice(&chunk[..count]);
                    if output.len() > MAX_BUFFER_BYTES {
                        return None;
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => stdout = None,
            }
        }
        if let Some(status) = child.try_wait().ok()?
            && stdout.is_none()
        {
            return status.success().then_some(output);
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())?;
        wait_for_pipes(
            stdin.as_ref(),
            stdout.as_ref(),
            remaining.min(Duration::from_millis(5)),
        );
    }
}

fn wait_for_pipes(stdin: Option<&ChildStdin>, stdout: Option<&ChildStdout>, timeout: Duration) {
    let mut fds: Vec<libc::pollfd> = stdin
        .map(|pipe| (pipe.as_raw_fd(), libc::POLLOUT))
        .into_iter()
        .chain(stdout.map(|pipe| (pipe.as_raw_fd(), libc::POLLIN)))
        .map(|(fd, events)| libc::pollfd {
            fd,
            events,
            revents: 0,
        })
        .collect();
    let timeout = timeout.as_millis().max(1) as libc::c_int;
    unsafe {
        libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout);
    }
}

fn copy_via_windows_clipboard(text: &str) -> bool {
    let id = pi_durable::ai::utils::uuid::uuidv7(None).unwrap_or_default();
    let path = std::env::temp_dir().join(format!("pi-wsl-clip-{id}.txt"));
    let copied = (|| {
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .and_then(|mut file| file.write_all(text.as_bytes()))
            .ok()?;
        let windows_path = run_clipboard_command(
            "wslpath",
            &["-w", &path.to_string_lossy()],
            None,
            Duration::from_millis(1000),
        )?;
        let windows_path = js_trim(&String::from_utf8_lossy(&windows_path)).to_owned();
        if windows_path.is_empty() {
            return None;
        }
        let script = format!(
            "Set-Clipboard -Value ([System.IO.File]::ReadAllText('{}', [System.Text.Encoding]::UTF8))",
            windows_path.replace('\'', "''")
        );
        run_clipboard_command(
            "powershell.exe",
            &["-NoProfile", "-Command", &script],
            None,
            Duration::from_millis(5000),
        )
    })()
    .is_some();
    let _ = std::fs::remove_file(&path);
    copied
}

pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let linux = cfg!(target_os = "linux");
    let mut commands: Vec<(&str, &[&str])> = Vec::new();
    if cfg!(target_os = "macos") {
        commands.push(("pbcopy", &[]));
    } else {
        if set("TERMUX_VERSION") {
            commands.push(("termux-clipboard-set", &[]));
        }
        if set("WAYLAND_DISPLAY") {
            commands.push(("wl-copy", &[]));
        }
        if set("DISPLAY") {
            commands.push(("xclip", &["-selection", "clipboard"]));
            commands.push(("xsel", &["--clipboard", "--input"]));
        }
    }
    let mut copied = commands.into_iter().any(|(command, args)| {
        run_clipboard_command(command, args, Some(text), Duration::from_millis(5000)).is_some()
    });
    let mut osc52_emitted = false;
    if !copied && linux && is_wsl() {
        if set("WT_SESSION") {
            osc52_emitted = emit_osc52(text);
        }
        copied = osc52_emitted || copy_via_windows_clipboard(text);
    }
    let headless = linux && !set("DISPLAY") && !set("WAYLAND_DISPLAY") && !set("TERMUX_VERSION");
    let mut oversized = false;
    if !osc52_emitted && (is_remote_session() || (!copied && headless)) {
        if emit_osc52(text) {
            copied = true;
        } else {
            oversized = true;
        }
    }
    if copied {
        return Ok(());
    }
    if oversized {
        return Err("Clipboard unavailable: text exceeds the OSC 52 size limit".to_owned());
    }
    if set("TERMUX_VERSION") {
        return Err(
            "Clipboard unavailable: install the Termux:API app and `termux-api` package".to_owned(),
        );
    }
    if linux {
        if set("WAYLAND_DISPLAY") {
            return Err(
                "Clipboard unavailable: install `wl-clipboard` (`wl-copy`) or check Wayland access"
                    .to_owned(),
            );
        }
        if set("DISPLAY") {
            return Err(
                "Clipboard unavailable: install `xclip` or `xsel`, or check X11 access".to_owned(),
            );
        }
    }
    Err("Clipboard unavailable".to_owned())
}
