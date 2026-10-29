use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::num::NonZeroU16;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::json;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SIDES: [&str; 2] = ["upstream", "pi-rs"];
const ROWS: u16 = 30;
const COLS: u16 = 100;
const QUIET: Duration = Duration::from_millis(1000);
const STEP: Duration = Duration::from_secs(30);
const CASE: Duration = Duration::from_secs(150);
const KEY_GAP: Duration = Duration::from_millis(20);
const SHOWN_ROWS: usize = 8;
const DEVICE_ATTRIBUTES: &[u8] = b"\x1b[?1;2c";

enum Step {
    Type(&'static str),
    Key(&'static str),
    Resize(u16, u16),
    Shows(&'static str, &'static str),
    Lacks(&'static str, &'static str),
}

struct Case {
    name: &'static str,
    extension: Option<&'static str>,
    replies: &'static [&'static str],
    steps: &'static [Step],
}

const CASES: &[Case] = &[
    Case {
        name: "startup",
        extension: None,
        replies: &["Fake reply to the first prompt."],
        steps: &[
            Step::Shows("started", "fake-test"),
            Step::Type("hello"),
            Step::Shows("typed", "hello"),
            Step::Key("\r"),
            Step::Shows("replied", "Fake reply to the first prompt."),
        ],
    },
    Case {
        name: "notify",
        extension: Some("session-name.ts"),
        replies: &[],
        steps: &[
            Step::Shows("started", "fake-test"),
            Step::Type("/session-name parity"),
            Step::Key("\r"),
            Step::Shows("notified", "Session named: parity"),
        ],
    },
    Case {
        name: "dialog",
        extension: Some("commands.ts"),
        replies: &[],
        steps: &[
            Step::Shows("started", "fake-test"),
            Step::Type("/commands"),
            Step::Key("\r"),
            Step::Shows("select", "Available Commands"),
            Step::Key("\x1b[B"),
            Step::Shows("moved", "Available Commands"),
            Step::Key("\r"),
            Step::Shows("confirm", "View source path?"),
            Step::Key("\r"),
            Step::Shows("answered", "commands.ts"),
            Step::Type("/commands"),
            Step::Key("\r"),
            Step::Shows("select-again", "Available Commands"),
            Step::Key("\x1b"),
            Step::Lacks("cancelled", "Available Commands"),
        ],
    },
    Case {
        name: "status",
        extension: Some("status-line.ts"),
        replies: &["Fake reply for the status line."],
        steps: &[
            Step::Shows("ready", "Ready"),
            Step::Type("hi"),
            Step::Key("\r"),
            Step::Shows("turn-done", "Turn 1 complete"),
        ],
    },
    Case {
        name: "widget-resize",
        extension: Some("widget-placement.ts"),
        replies: &[],
        steps: &[
            Step::Shows("started", "Below editor widget"),
            Step::Resize(24, 70),
            Step::Shows("narrow", "Below editor widget"),
            Step::Resize(ROWS, COLS),
            Step::Shows("restored", "Below editor widget"),
        ],
    },
    Case {
        name: "overlay",
        extension: Some("overlay-test.ts"),
        replies: &[],
        steps: &[
            Step::Shows("started", "fake-test"),
            Step::Type("/overlay-test"),
            Step::Key("\r"),
            Step::Shows("open", "Overlay Test"),
            Step::Type("abc"),
            Step::Shows("typed", "abc"),
            Step::Key("\r"),
            Step::Shows("closed", "Search: \"abc\""),
        ],
    },
    Case {
        name: "shortcut",
        extension: Some("plan-mode/index.ts"),
        replies: &[],
        steps: &[
            Step::Shows("started", "fake-test"),
            Step::Key("\x1b\x10"),
            Step::Shows("enabled", "Plan mode enabled"),
            Step::Key("\x1b\x10"),
            Step::Shows("disabled", "Plan mode disabled"),
        ],
    },
    Case {
        name: "message-renderer",
        extension: Some("message-renderer.ts"),
        replies: &[],
        steps: &[
            Step::Shows("started", "fake-test"),
            Step::Type("/status warn disk almost full"),
            Step::Key("\r"),
            Step::Shows("rendered", "[WARN]"),
        ],
    },
];

#[derive(Default)]
struct Answers {
    pending: Vec<u8>,
    title: String,
}

impl vt100::Callbacks for Answers {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        first: Option<u8>,
        _: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let param = params.first().and_then(|param| param.first()).copied();
        match (first, c, param.unwrap_or(0)) {
            (None, 'c', 0) => self.pending.extend_from_slice(DEVICE_ATTRIBUTES),
            (None, 'n', 5) => self.pending.extend_from_slice(b"\x1b[0n"),
            (None, 'n', 6) => {
                let (row, col) = screen.cursor_position();
                self.pending
                    .extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
            _ => {}
        }
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).into_owned();
    }
}

struct Terminal {
    parser: vt100::Parser<Answers>,
    raw: Vec<u8>,
    last: Instant,
}

#[derive(PartialEq)]
struct Screen {
    rows: Vec<String>,
    styled: Vec<String>,
    cursor: (u16, u16),
    cursor_hidden: bool,
    alternate: bool,
    title: String,
}

fn style(cell: &vt100::Cell, blank: bool) -> String {
    let color = |name: &str, color: vt100::Color| match color {
        vt100::Color::Default => String::new(),
        vt100::Color::Idx(index) => format!(" {name}{index}"),
        vt100::Color::Rgb(r, g, b) => format!(" {name}#{r:02x}{g:02x}{b:02x}"),
    };
    let mut out = color("bg", cell.bgcolor());
    if !blank || cell.inverse() || cell.underline() {
        out.push_str(&color("fg", cell.fgcolor()));
    }
    for (on, shows_on_blank, name) in [
        (cell.bold(), false, " bold"),
        (cell.dim(), false, " dim"),
        (cell.italic(), false, " italic"),
        (cell.underline(), true, " underline"),
        (cell.inverse(), true, " inverse"),
    ] {
        if on && (shows_on_blank || !blank) {
            out.push_str(name);
        }
    }
    out
}

impl Screen {
    fn of(terminal: &Terminal) -> Screen {
        let screen = terminal.parser.screen();
        let (rows, cols) = screen.size();
        let mut plain = Vec::new();
        let mut styled = Vec::new();
        for row in 0..rows.get() {
            let mut cells: Vec<(&str, String)> = (0..cols.get())
                .filter_map(|col| screen.cell(row, col))
                .filter(|cell| !cell.is_wide_continuation())
                .map(|cell| {
                    let contents = cell.contents();
                    let blank = contents.trim().is_empty();
                    (if blank { " " } else { contents }, style(cell, blank))
                })
                .collect();
            while cells
                .last()
                .is_some_and(|(contents, style)| *contents == " " && style.is_empty())
            {
                cells.pop();
            }
            let mut marked = String::new();
            let mut current = "";
            for (contents, style) in &cells {
                if style != current {
                    marked.push_str(&format!("⟦{}⟧", style.trim_start()));
                    current = style;
                }
                marked.push_str(contents);
            }
            let text: String = cells.iter().map(|(contents, _)| *contents).collect();
            plain.push(text.trim_end().to_owned());
            styled.push(marked);
        }
        Screen {
            rows: plain,
            styled,
            cursor: screen.cursor_position(),
            cursor_hidden: screen.hide_cursor(),
            alternate: screen.alternate_screen(),
            title: terminal.parser.callbacks().title.clone(),
        }
    }

    fn text(&self) -> String {
        self.rows.join("\n")
    }

    fn render(&self) -> String {
        let cursor = if self.cursor_hidden {
            "hidden"
        } else {
            "shown"
        };
        let mut out = format!(
            "cursor {},{} {cursor}; alternate screen {}; title {:?}\n",
            self.cursor.0, self.cursor.1, self.alternate, self.title
        );
        for (row, (text, marked)) in self.rows.iter().zip(&self.styled).enumerate() {
            out.push_str(&format!("{row:>2} |{text}\n   |{marked}\n"));
        }
        out
    }
}

fn locked(terminal: &Mutex<Terminal>) -> MutexGuard<'_, Terminal> {
    terminal
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn pump(mut master: File, terminal: Arc<Mutex<Terminal>>) {
    let mut chunk = [0u8; 65536];
    loop {
        let read = match master.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let answer = {
            let mut terminal = locked(&terminal);
            terminal.raw.extend_from_slice(&chunk[..read]);
            terminal.parser.process(&chunk[..read]);
            terminal.last = Instant::now();
            std::mem::take(&mut terminal.parser.callbacks_mut().pending)
        };
        if !answer.is_empty() && master.write_all(&answer).is_err() {
            break;
        }
    }
}

fn dimensions(rows: u16, cols: u16) -> Result<(NonZeroU16, NonZeroU16)> {
    match (NonZeroU16::new(rows), NonZeroU16::new(cols)) {
        (Some(rows), Some(cols)) => Ok((rows, cols)),
        _ => Err(format!("a terminal of {rows}x{cols} cells").into()),
    }
}

fn winsize(rows: u16, cols: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

struct Pty {
    pid: libc::pid_t,
    master: File,
    terminal: Arc<Mutex<Terminal>>,
    reader: Option<JoinHandle<()>>,
    exited: Option<String>,
}

impl Pty {
    fn spawn(command: &mut Command) -> Result<Pty> {
        let (mut master, mut slave) = (-1, -1);
        let size = winsize(ROWS, COLS);
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
            return Err(format!("openpty: {}", std::io::Error::last_os_error()).into());
        }
        let (master, slave) =
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        command
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let pid = libc::pid_t::try_from(command.spawn()?.id())?;
        let (rows, cols) = dimensions(ROWS, COLS)?;
        let terminal = Arc::new(Mutex::new(Terminal {
            parser: vt100::Parser::new_with_callbacks(rows, cols, 0, Answers::default()),
            raw: Vec::new(),
            last: Instant::now(),
        }));
        let master = File::from(master);
        let reader = master.try_clone()?;
        let shared = Arc::clone(&terminal);
        let reader = std::thread::spawn(move || pump(reader, shared));
        Ok(Pty {
            pid,
            master,
            terminal,
            reader: Some(reader),
            exited: None,
        })
    }

    fn alive(&mut self) -> bool {
        if self.exited.is_some() {
            return false;
        }
        let mut status = 0;
        if unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) } == self.pid {
            self.exited = Some(if libc::WIFEXITED(status) {
                format!("exited {}", libc::WEXITSTATUS(status))
            } else {
                format!("was killed by signal {}", libc::WTERMSIG(status))
            });
            return false;
        }
        true
    }

    fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.master.write_all(bytes)?;
        Ok(())
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        let (height, width) = dimensions(rows, cols)?;
        let mut terminal = locked(&self.terminal);
        terminal.parser.set_size(height, width);
        let size = winsize(rows, cols);
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &size) } != 0 {
            return Err(format!("TIOCSWINSZ: {}", std::io::Error::last_os_error()).into());
        }
        Ok(())
    }

    fn settled(&mut self, since: Instant, wanted: impl Fn(&str) -> bool) -> (Screen, bool) {
        let deadline = Instant::now() + STEP;
        loop {
            let alive = self.alive();
            {
                let terminal = locked(&self.terminal);
                let screen = Screen::of(&terminal);
                let quiet = terminal.last > since && terminal.last.elapsed() >= QUIET;
                if (quiet || !alive) && wanted(&screen.text()) {
                    return (screen, true);
                }
                if !alive || Instant::now() > deadline {
                    return (screen, false);
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn raw(&self) -> Vec<u8> {
        locked(&self.terminal).raw.clone()
    }

    fn stop(&mut self) -> Option<String> {
        let mut problem = None;
        unsafe {
            libc::kill(-self.pid, libc::SIGKILL);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut status = 0;
            while unsafe { libc::waitpid(-self.pid, &mut status, libc::WNOHANG) } > 0 {}
            if unsafe { libc::kill(-self.pid, 0) } != 0 {
                break;
            }
            if Instant::now() > deadline {
                problem = Some(format!(
                    "process group {} still alive 10s after SIGKILL",
                    self.pid
                ));
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if let Some(reader) = self.reader.take() {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !reader.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if reader.is_finished() {
                let _ = reader.join();
            } else if problem.is_none() {
                problem = Some("a process outside the group kept the terminal open".into());
            }
        }
        problem
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if self.reader.is_some() {
            self.stop();
        }
    }
}

type Model = Arc<Mutex<HashMap<String, (Vec<String>, usize)>>>;

fn request_label(stream: &mut TcpStream) -> Result<String> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line
        .split_whitespace()
        .nth(1)
        .ok_or("no request path")?
        .to_owned();
    let mut length = 0;
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse()?;
        }
    }
    reader.read_exact(&mut vec![0; length])?;
    Ok(path
        .trim_start_matches('/')
        .strip_suffix("/v1/chat/completions")
        .ok_or_else(|| format!("unexpected request path {path}"))?
        .to_owned())
}

fn answer(mut stream: TcpStream, model: &Model) -> Result<()> {
    let label = request_label(&mut stream)?;
    let reply = {
        let mut model = model.lock().map_err(|_| "model lock poisoned")?;
        let (replies, asked) = model
            .get_mut(&label)
            .ok_or_else(|| format!("request for unknown case {label}"))?;
        *asked += 1;
        replies.get(*asked - 1).cloned()
    };
    let (status, kind, body) = match reply {
        Some(text) => (
            "200 OK",
            "text/event-stream",
            [
                json!({"id": "c", "choices": [{"index": 0, "delta": {"role": "assistant", "content": text}, "finish_reason": null}]}),
                json!({"id": "c", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
            ]
            .iter()
            .map(|chunk| format!("data: {chunk}\n\n"))
            .chain(["data: [DONE]\n\n".to_owned()])
            .collect(),
        ),
        None => (
            "400 Bad Request",
            "application/json",
            json!({"error": {"message": "no scripted reply left"}}).to_string(),
        ),
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    Ok(())
}

fn env_path(name: &str) -> Result<PathBuf> {
    match std::env::var_os(name) {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Err(format!("set {name}").into()),
    }
}

struct Checkpoint {
    name: &'static str,
    screen: Screen,
}

struct Run {
    checkpoints: Vec<Checkpoint>,
    problems: Vec<String>,
    raw: Vec<u8>,
}

struct Setup<'a> {
    fixtures: &'a Path,
    base_url: String,
    root: &'a Path,
}

fn command(bin: &Path, setup: &Setup, case: &Case) -> Result<Command> {
    let root = setup.root;
    for dir in ["home", "agent", "cwd"] {
        std::fs::create_dir_all(root.join(dir))?;
    }
    let models = json!({"providers": {"fake": {
        "baseUrl": setup.base_url,
        "apiKey": "sk-test",
        "api": "openai-completions",
        "models": [{"id": "fake-test", "contextWindow": 200000, "maxTokens": 8192}],
    }}});
    std::fs::write(root.join("agent/models.json"), models.to_string())?;
    let mut command = Command::new(bin);
    command
        .args(["--provider", "fake", "--model", "fake-test"])
        .current_dir(root.join("cwd"))
        .env_clear()
        .env("HOME", root.join("home"))
        .env("PI_CODING_AGENT_DIR", root.join("agent"))
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("TERM", "xterm-256color")
        .env("TZ", "UTC")
        .env("PI_OFFLINE", "1")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_TELEMETRY", "0")
        .envs(std::env::var_os("PI_EXTENSION_HOST").map(|host| ("PI_EXTENSION_HOST", host)));
    if let Some(extension) = case.extension {
        let path = setup.fixtures.join("upstream").join(extension);
        if !path.is_file() {
            return Err(format!("{} is not a fixture extension", path.display()).into());
        }
        command.arg("-e").arg(path);
    }
    Ok(command)
}

fn drive(side: &str, bin: &Path, setup: &Setup, case: &Case) -> Run {
    let mut run = Run {
        checkpoints: Vec::new(),
        problems: Vec::new(),
        raw: Vec::new(),
    };
    let mut since = Instant::now();
    let mut pty = match command(bin, setup, case).and_then(|mut command| Pty::spawn(&mut command)) {
        Ok(pty) => pty,
        Err(error) => {
            run.problems
                .push(format!("{side}: could not start: {error}"));
            return run;
        }
    };
    let started = Instant::now();
    for step in case.steps {
        if started.elapsed() > CASE {
            run.problems
                .push(format!("{side}: the case ran past its {CASE:?} budget"));
            break;
        }
        let acted = match step {
            Step::Type(text) => text.chars().try_for_each(|key| {
                let mut buf = [0u8; 4];
                std::thread::sleep(KEY_GAP);
                since = Instant::now();
                pty.send(key.encode_utf8(&mut buf).as_bytes())
            }),
            Step::Key(bytes) => {
                since = Instant::now();
                pty.send(bytes.as_bytes())
            }
            Step::Resize(rows, cols) => {
                since = Instant::now();
                pty.resize(*rows, *cols)
            }
            Step::Shows(..) | Step::Lacks(..) => Ok(()),
        };
        if let Err(error) = acted {
            run.problems
                .push(format!("{side}: could not act on the terminal: {error}"));
            break;
        }
        let (name, text, shows) = match step {
            Step::Shows(name, text) => (*name, *text, true),
            Step::Lacks(name, text) => (*name, *text, false),
            Step::Type(text) => {
                if pty.settled(since, |_| true).1 {
                    continue;
                }
                run.problems.push(format!(
                    "{side}: the screen did not settle after typing {text:?}"
                ));
                break;
            }
            Step::Key(_) | Step::Resize(..) => continue,
        };
        let (screen, reached) = pty.settled(since, |screen| screen.contains(text) == shows);
        run.checkpoints.push(Checkpoint { name, screen });
        if !reached {
            let what = if shows { "show" } else { "clear" };
            let why = match &pty.exited {
                Some(exit) => format!("it {exit}"),
                None => format!("{STEP:?} passed"),
            };
            run.problems.push(format!(
                "{side}: did not settle to {what} {text:?} at {name} ({why})"
            ));
            break;
        }
    }
    if let Some(exit) = &pty.exited {
        run.problems
            .push(format!("{side}: {exit} before the case ended"));
    }
    run.problems.extend(pty.stop());
    run.raw = pty.raw();
    run
}

#[derive(Default)]
struct Reported {
    rows: HashSet<(String, String)>,
    states: HashMap<&'static str, (String, String)>,
}

impl Reported {
    fn rows(&mut self, name: &str, theirs: &[String], ours: &[String], out: &mut Vec<String>) {
        let empty = String::new();
        let differing: Vec<usize> = (0..theirs.len().max(ours.len()))
            .filter(|row| theirs.get(*row) != ours.get(*row))
            .collect();
        let mut new: Vec<(usize, (String, String))> = Vec::new();
        for row in &differing {
            let pair = (
                theirs.get(*row).unwrap_or(&empty).clone(),
                ours.get(*row).unwrap_or(&empty).clone(),
            );
            if !self.rows.contains(&pair) && !new.iter().any(|(_, seen)| *seen == pair) {
                new.push((*row, pair));
            }
        }
        out.push(format!(
            "{name}: {} rows differ, {} not shown before",
            differing.len(),
            new.len()
        ));
        for (row, pair) in new.into_iter().take(SHOWN_ROWS) {
            out.push(format!("{name} row {row:>2} upstream |{}", pair.0));
            out.push(format!("{name} row {row:>2} pi-rs    |{}", pair.1));
            self.rows.insert(pair);
        }
    }

    fn state(
        &mut self,
        name: &str,
        what: &'static str,
        pair: (String, String),
        out: &mut Vec<String>,
    ) {
        if pair.0 != pair.1 && self.states.get(what) != Some(&pair) {
            out.push(format!(
                "{name}: {what} upstream {}, pi-rs {}",
                pair.0, pair.1
            ));
        }
        self.states.insert(what, pair);
    }
}

fn cursor(screen: &Screen) -> String {
    let shown = if screen.cursor_hidden {
        "hidden"
    } else {
        "shown"
    };
    format!("{},{} {shown}", screen.cursor.0, screen.cursor.1)
}

fn compare(theirs: &Run, ours: &Run) -> Vec<String> {
    let mut out = Vec::new();
    let mut reported = Reported::default();
    for at in 0..theirs.checkpoints.len().max(ours.checkpoints.len()) {
        match (theirs.checkpoints.get(at), ours.checkpoints.get(at)) {
            (Some(a), Some(b)) if a.screen == b.screen => {}
            (Some(a), Some(b)) => {
                let (x, y) = (&a.screen, &b.screen);
                if x.rows != y.rows {
                    reported.rows(a.name, &x.rows, &y.rows, &mut out);
                } else if x.styled != y.styled {
                    out.push(format!("{}: same text, different styling", a.name));
                    reported.rows(a.name, &x.styled, &y.styled, &mut out);
                }
                reported.state(a.name, "cursor", (cursor(x), cursor(y)), &mut out);
                reported.state(
                    a.name,
                    "alternate screen",
                    (x.alternate.to_string(), y.alternate.to_string()),
                    &mut out,
                );
                reported.state(
                    a.name,
                    "title",
                    (format!("{:?}", x.title), format!("{:?}", y.title)),
                    &mut out,
                );
            }
            (Some(a), None) => out.push(format!("{}: pi-rs never reached it", a.name)),
            (None, Some(b)) => out.push(format!("{}: upstream never reached it", b.name)),
            (None, None) => {}
        }
    }
    out
}

fn keep(dir: &Path, case: &Case, side: &str, run: &Run) -> Result<()> {
    let dir = dir.join(case.name).join(side);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("pty.raw"), &run.raw)?;
    let mut screens = String::new();
    for checkpoint in &run.checkpoints {
        screens.push_str(&format!(
            "== {}\n{}",
            checkpoint.name,
            checkpoint.screen.render()
        ));
    }
    for problem in &run.problems {
        screens.push_str(&format!("!! {problem}\n"));
    }
    std::fs::write(dir.join("screens.txt"), screens)?;
    Ok(())
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.0.exists()
            && let Err(error) = std::fs::remove_dir_all(&self.0)
        {
            eprintln!("could not remove {}: {error}", self.0.display());
        }
    }
}

fn main() -> Result<()> {
    let bins = [env_path("PI_UPSTREAM_BIN")?, env_path("PI_RS_BIN")?];
    let fixtures = env_path("PI_PARITY_FIXTURES")?;
    let kept = std::env::var_os("PI_TUI_PARITY_KEEP").map(PathBuf::from);
    let only: Vec<String> = std::env::args().skip(1).collect();
    let cases: Vec<&Case> = CASES
        .iter()
        .filter(|case| only.is_empty() || only.iter().any(|name| name == case.name))
        .collect();
    if cases.is_empty() {
        return Err(format!("no case matches {}", only.join(" ")).into());
    }
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        return Err(format!(
            "PR_SET_CHILD_SUBREAPER: {}",
            std::io::Error::last_os_error()
        )
        .into());
    }

    let model: Model = Arc::new(Mutex::new(HashMap::new()));
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let served = Arc::clone(&model);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let model = Arc::clone(&served);
            std::thread::spawn(move || {
                if let Err(error) = answer(stream, &model) {
                    eprintln!("fake model: {error}");
                }
            });
        }
    });

    let scratch = Scratch(std::env::temp_dir().join(format!("tui-parity-{}", std::process::id())));
    let mut failed = Vec::new();
    for case in &cases {
        let root = scratch.0.join(case.name);
        let setup = Setup {
            fixtures: &fixtures,
            base_url: format!("http://127.0.0.1:{port}/{}/v1", case.name),
            root: &root,
        };
        let mut runs = Vec::new();
        for (side, bin) in SIDES.iter().zip(&bins) {
            model.lock().map_err(|_| "model lock poisoned")?.insert(
                case.name.to_owned(),
                (
                    case.replies.iter().map(|&reply| reply.to_owned()).collect(),
                    0,
                ),
            );
            if root.exists() {
                std::fs::remove_dir_all(&root)?;
            }
            let mut run = drive(side, bin, &setup, case);
            let asked = model
                .lock()
                .map_err(|_| "model lock poisoned")?
                .get(case.name)
                .map_or(0, |(_, asked)| *asked);
            if asked != case.replies.len() {
                run.problems.push(format!(
                    "{side}: sent {asked} model requests, the case scripts {}",
                    case.replies.len()
                ));
            }
            if let Some(dir) = &kept {
                keep(dir, case, side, &run)?;
            }
            runs.push(run);
        }
        let differences = compare(&runs[0], &runs[1]);
        let problems: Vec<&String> = runs.iter().flat_map(|run| &run.problems).collect();
        if differences.is_empty() && problems.is_empty() {
            println!("{}: same {} screens", case.name, runs[0].checkpoints.len());
            continue;
        }
        println!("{}: DIFFERS", case.name);
        for line in problems.into_iter().chain(&differences) {
            println!("  {line}");
        }
        failed.push(case.name);
    }
    println!(
        "tui parity: {} of {} cases same",
        cases.len() - failed.len(),
        cases.len()
    );
    if !failed.is_empty() {
        return Err(format!(
            "{} of {} cases differ: {} (PI_TUI_PARITY_KEEP=<dir> keeps each checkpoint's screens and each side's raw terminal output)",
            failed.len(),
            cases.len(),
            failed.join(", ")
        )
        .into());
    }
    Ok(())
}
