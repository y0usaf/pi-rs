use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const WAIT: Duration = Duration::from_secs(20);

fn fail<T>(message: String) -> Result<T> {
    Err(message.into())
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn binary() -> Result<PathBuf> {
    let path = match std::env::var("PI_RS_BIN") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => return fail("set PI_RS_BIN to the built pi-rs".into()),
    };
    if !executable(&path) {
        return fail(format!("{} is not an executable file", path.display()));
    }
    Ok(std::fs::canonicalize(path)?)
}

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().join(format!("interactive-e2e-{}", std::process::id()));
        for dir in ["home", "agent", "cwd"] {
            std::fs::create_dir_all(root.join(dir))?;
        }
        Ok(Self { root })
    }

    fn command(&self, bin: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(bin);
        command
            .args(args)
            .current_dir(self.root.join("cwd"))
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("PI_CODING_AGENT_DIR", self.root.join("agent"))
            .env("PATH", "/nonexistent")
            .env("TERM", "xterm-256color");
        command
    }
}

fn plain(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']' | 'P' | '_' | '^' | 'X') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

struct Tui {
    child: Child,
    input: File,
    output: Arc<Mutex<Vec<u8>>>,
    seen: usize,
}

impl Tui {
    fn start(command: &mut Command) -> Result<Self> {
        let (mut master, mut slave) = (-1, -1);
        let size = libc::winsize {
            ws_row: 40,
            ws_col: 160,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let opened = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        };
        if opened != 0 {
            return fail(format!("openpty: {}", std::io::Error::last_os_error()));
        }
        let (master, slave) =
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        let input = File::from(master.try_clone()?);
        let mut reader = File::from(master.try_clone()?);
        drop(master);
        let (stdin, stdout, stderr) = (slave.try_clone()?, slave.try_clone()?, slave.try_clone()?);
        drop(slave);
        let child = command
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()?;
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&output);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(read) = reader.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                sink.lock().unwrap().extend_from_slice(&chunk[..read]);
            }
        });
        Ok(Self {
            child,
            input,
            output,
            seen: 0,
        })
    }

    fn screen(&self) -> String {
        plain(&self.output.lock().unwrap())
    }

    fn tail(&self) -> String {
        let screen = self.screen();
        let start = screen.len().saturating_sub(3000);
        let start = (start..screen.len())
            .find(|i| screen.is_char_boundary(*i))
            .unwrap_or(screen.len());
        screen[start..].to_owned()
    }

    fn running(&mut self, step: &str) -> Result<()> {
        match self.child.try_wait()? {
            None => Ok(()),
            Some(status) => fail(format!(
                "pi-rs exited {status} {step}; screen:\n{}",
                self.tail()
            )),
        }
    }

    fn expect(&mut self, text: &str, step: &str) -> Result<()> {
        let deadline = Instant::now() + WAIT;
        loop {
            let screen = self.screen();
            if let Some(at) = screen[self.seen..].find(text) {
                self.seen += at + text.len();
                return self.running(step);
            }
            self.running(step)?;
            if Instant::now() > deadline {
                return fail(format!(
                    "no {text:?} {step} within {WAIT:?}; screen:\n{}",
                    self.tail()
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn keys(&mut self, keys: &str) -> Result<()> {
        for key in keys.chars() {
            let mut buf = [0u8; 4];
            self.input.write_all(key.encode_utf8(&mut buf).as_bytes())?;
            std::thread::sleep(Duration::from_millis(30));
        }
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }

    fn exit(&mut self) -> Result<ExitStatus> {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            if Instant::now() > deadline {
                return fail(format!(
                    "pi-rs still running {WAIT:?} after Ctrl+D; screen:\n{}",
                    self.tail()
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn headless_refusals(bin: &Path, scratch: &Scratch) -> Result<()> {
    for args in [
        &["-p", "hello"][..],
        &["--mode", "json", "hello"],
        &["--mode", "rpc"],
    ] {
        let out = scratch.command(bin, args).stdin(Stdio::null()).output()?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        if out.status.code() != Some(1) || !stderr.contains("No models available.") {
            return fail(format!(
                "pi-rs {} with no model exited {}, stderr: {stderr}",
                args.join(" "),
                out.status
            ));
        }
    }
    Ok(())
}

fn interactive_without_model(bin: &Path, scratch: &Scratch) -> Result<()> {
    let mut tui = Tui::start(&mut scratch.command(bin, &[]))?;
    tui.expect("unknown", "in the footer")?;
    tui.expect("Warning: No models available.", "at startup")?;
    std::thread::sleep(Duration::from_millis(500));
    tui.running("after startup")?;

    tui.keys("hello\r")?;
    tui.expect(
        "Error: No API key found for the selected model.",
        "after a prompt",
    )?;
    tui.expect("Use /login to log into a provider", "after a prompt")?;
    std::thread::sleep(Duration::from_millis(500));
    tui.running("after a prompt")?;

    tui.keys("/login\r")?;
    tui.expect("Select authentication method:", "after /login")?;

    tui.keys("\x1b")?;
    tui.keys("\x04")?;
    let status = tui.exit()?;
    if !status.success() {
        return fail(format!(
            "pi-rs exited {status} after Ctrl+D; screen:\n{}",
            tui.tail()
        ));
    }
    Ok(())
}

fn authorization(stream: &mut TcpStream) -> Result<String> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let (mut length, mut authorization) = (0, String::new());
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse()?,
                "authorization" => authorization = value.trim().to_owned(),
                _ => {}
            }
        }
    }
    reader.read_exact(&mut vec![0; length])?;
    Ok(authorization)
}

fn reply(mut stream: TcpStream, seen: &Mutex<Vec<String>>) -> Result<()> {
    seen.lock().unwrap().push(authorization(&mut stream)?);
    stream.write_all(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n",
    )?;
    for event in [
        r#"{"id":"c1","choices":[{"index":0,"delta":{"role":"assistant","content":"pong from the fake provider"}}]}"#,
        r#"{"id":"c1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        r#"{"id":"c1","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#,
        "[DONE]",
    ] {
        let chunk = format!("data: {event}\n\n");
        write!(stream, "{:x}\r\n{chunk}\r\n", chunk.len())?;
    }
    stream.write_all(b"0\r\n\r\n")?;
    Ok(stream.flush()?)
}

fn login_then_chat(bin: &Path, scratch: &Scratch) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Err(error) = reply(stream, &served) {
                eprintln!("fake provider: {error}");
            }
        }
    });
    std::fs::write(
        scratch.root.join("agent").join("models.json"),
        format!(r#"{{"providers": {{"deepseek": {{"baseUrl": "http://127.0.0.1:{port}"}}}}}}"#),
    )?;
    let mut tui = Tui::start(&mut scratch.command(bin, &[]))?;
    tui.expect(
        "Warning: No models available.",
        "with a keyless provider configured",
    )?;
    tui.keys("/login deepseek\r")?;
    tui.expect("Enter DeepSeek API key", "after /login deepseek")?;
    tui.keys("sk-e2e\r")?;
    tui.expect(
        "Saved API key for DeepSeek. Selected deepseek-v4-pro.",
        "after saving the key",
    )?;
    tui.keys("hi\r")?;
    tui.expect("pong from the fake provider", "after a prompt")?;
    let seen = seen.lock().unwrap().clone();
    if seen != ["Bearer sk-e2e"] {
        return fail(format!("the fake provider saw authorization {seen:?}"));
    }
    tui.keys("\x04")?;
    let status = tui.exit()?;
    if !status.success() {
        return fail(format!(
            "pi-rs exited {status} after Ctrl+D; screen:\n{}",
            tui.tail()
        ));
    }
    Ok(())
}

fn main() -> Result<()> {
    let bin = binary()?;
    let scratch = Scratch::new()?;
    headless_refusals(&bin, &scratch).map_err(|e| format!("print, json and rpc modes: {e}"))?;
    interactive_without_model(&bin, &scratch)
        .map_err(|e| format!("interactive without a model: {e}"))?;
    login_then_chat(&bin, &scratch).map_err(|e| format!("login, then chat: {e}"))?;
    std::fs::remove_dir_all(&scratch.root)?;
    println!("interactive e2e ok");
    Ok(())
}
