use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SIDES: [&str; 2] = ["upstream", "pi-rs"];
const WAIT: Duration = Duration::from_secs(30);
const QUIET: Duration = Duration::from_secs(3);
const SETTLED: &str = "pi.on(agent_settled)#1";
const STARTING: &str = "Startup is still in progress";

#[derive(Clone, Copy)]
enum Mode {
    Rpc,
    Tui,
}

#[derive(Clone, Copy)]
enum Queued {
    Steer(&'static str),
    FollowUp(&'static str),
}

struct Scenario {
    name: &'static str,
    mode: Mode,
    prompt: &'static str,
    queued: &'static [Queued],
    replies: &'static [&'static str],
}

const SCENARIOS: [Scenario; 3] = [
    Scenario {
        name: "rpc-queue",
        mode: Mode::Rpc,
        prompt: "hi",
        queued: &[Queued::Steer("steer text"), Queued::FollowUp("follow text")],
        replies: &["one", "two", "three"],
    },
    Scenario {
        name: "rpc-continue",
        mode: Mode::Rpc,
        prompt: "continue",
        queued: &[],
        replies: &["one", "two"],
    },
    Scenario {
        name: "tui-queue",
        mode: Mode::Tui,
        prompt: "hi",
        queued: &[Queued::Steer("queued text")],
        replies: &["one", "two"],
    },
];

#[derive(Default)]
struct Script {
    replies: &'static [&'static str],
    hold: bool,
    log: PathBuf,
    requests: Vec<Value>,
    marks: Vec<u64>,
}

#[derive(Default)]
struct Model {
    script: Mutex<Script>,
    changed: Condvar,
}

impl Model {
    fn lock(&self) -> Result<MutexGuard<'_, Script>> {
        self.script
            .lock()
            .map_err(|_| "script lock poisoned".into())
    }

    fn reset(&self, scenario: &Scenario, log: PathBuf) -> Result<()> {
        *self.lock()? = Script {
            replies: scenario.replies,
            hold: !scenario.queued.is_empty(),
            log,
            requests: Vec::new(),
            marks: Vec::new(),
        };
        Ok(())
    }

    fn release(&self) -> Result<()> {
        self.lock()?.hold = false;
        self.changed.notify_all();
        Ok(())
    }

    fn arrived(&self, count: usize) -> Result<()> {
        let (script, waited) = self
            .changed
            .wait_timeout_while(self.lock()?, WAIT, |script| script.requests.len() < count)
            .map_err(|_| "script lock poisoned")?;
        if waited.timed_out() {
            return Err(format!(
                "request {count} did not arrive within {WAIT:?} ({} arrived)",
                script.requests.len()
            )
            .into());
        }
        Ok(())
    }

    fn progress(&self) -> Result<Progress> {
        let script = self.lock()?;
        let logged = std::fs::metadata(&script.log).map_or(0, |meta| meta.len());
        Ok(Progress {
            served: script.requests.len(),
            scripted: script.replies.len(),
            logged,
            ended: script.requests.len() >= script.replies.len()
                && settled_after(&script.log, script.marks.last().copied().unwrap_or(0))?,
        })
    }

    fn requests(&self) -> Result<Vec<Value>> {
        Ok(self.lock()?.requests.clone())
    }
}

#[derive(PartialEq)]
struct Progress {
    served: usize,
    scripted: usize,
    logged: u64,
    ended: bool,
}

fn settle(model: &Model, mut idle: impl FnMut() -> Result<bool>) -> Result<()> {
    let deadline = Instant::now() + WAIT;
    let mut last = model.progress()?;
    let mut since = Instant::now();
    loop {
        let progress = model.progress()?;
        if progress != last {
            last = progress;
            since = Instant::now();
        }
        let stalled = last.served > 0 && since.elapsed() >= QUIET;
        if (last.ended || stalled) && idle()? {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!(
                "not idle within {WAIT:?}: {} of {} scripted requests arrived",
                last.served, last.scripted
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[derive(Deserialize)]
struct Record {
    id: u64,
    kind: String,
    #[serde(default)]
    path: Option<String>,
}

fn settled_after(log: &Path, mark: u64) -> Result<bool> {
    let text = match std::fs::read(log) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let start = usize::try_from(mark)?.min(text.len());
    let records: Vec<Record> = String::from_utf8_lossy(&text[start..])
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let calls: Vec<u64> = records
        .iter()
        .filter(|record| record.kind == "callback" && record.path.as_deref() == Some(SETTLED))
        .map(|record| record.id)
        .collect();
    Ok(records.iter().any(|record| {
        matches!(record.kind.as_str(), "return" | "resolve") && calls.contains(&record.id)
    }))
}

fn env_path(name: &str) -> Result<PathBuf> {
    match std::env::var_os(name) {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Err(format!("set {name}").into()),
    }
}

fn read_request(stream: &mut TcpStream) -> Result<(String, Value)> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line
        .split_whitespace()
        .nth(1)
        .ok_or("no request path")?
        .to_owned();
    let mut length = None;
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
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length.ok_or("no content-length")?];
    reader.read_exact(&mut body)?;
    Ok((path, serde_json::from_slice(&body)?))
}

fn stream_body(index: usize, text: &str) -> String {
    let id = format!("c{index}");
    [
        json!({"id": id, "choices": [{"index": 0, "delta": {"role": "assistant", "content": text}, "finish_reason": null}]}),
        json!({"id": id, "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
    ]
    .iter()
    .map(|chunk| format!("data: {chunk}\n\n"))
    .chain(["data: [DONE]\n\n".to_owned()])
    .collect()
}

fn serve(mut stream: TcpStream, model: &Model) -> Result<()> {
    let (path, body) = read_request(&mut stream)?;
    if path != "/v1/chat/completions" {
        return Err(format!("unexpected request path {path}").into());
    }
    let next = {
        let mut script = model.lock()?;
        let mark = std::fs::metadata(&script.log).map_or(0, |meta| meta.len());
        script.requests.push(body);
        script.marks.push(mark);
        let index = script.requests.len() - 1;
        model.changed.notify_all();
        let script = if index == 0 {
            model
                .changed
                .wait_while(script, |script| script.hold)
                .map_err(|_| "script lock poisoned")?
        } else {
            script
        };
        script
            .replies
            .get(index)
            .map(|text| stream_body(index, text))
    };
    let (status, kind, body) = match next {
        Some(body) => ("200 OK", "text/event-stream", body),
        None => (
            "400 Bad Request",
            "application/json",
            json!({"error": {"message": "no scripted turn left"}}).to_string(),
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

fn command(bin: &Path, root: &Path, mode: Mode) -> Command {
    let mut command = Command::new(bin);
    command
        .args([
            "--provider",
            "fake",
            "--model",
            "fake-test",
            "--session-dir",
        ])
        .arg(root.join("sessions"))
        .arg("-e")
        .arg(root.join("wrap.ts"))
        .current_dir(root.join("cwd"))
        .env_clear()
        .env("HOME", root.join("home"))
        .env("PI_CODING_AGENT_DIR", root.join("agent"))
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("PI_OFFLINE", "1")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_TELEMETRY", "0")
        .env("PI_PARITY_LOG", root.join("extension.log"))
        .envs(std::env::var_os("PI_EXTENSION_HOST").map(|host| ("PI_EXTENSION_HOST", host)));
    match mode {
        Mode::Rpc => {
            command.args(["--mode", "rpc"]);
        }
        Mode::Tui => {
            command.env("TERM", "xterm-256color");
        }
    }
    command
}

fn prepare(root: &Path, base_url: &str, extension: &Path, probe: &Path) -> Result<()> {
    for dir in ["home", "agent", "cwd", "sessions"] {
        std::fs::create_dir_all(root.join(dir))?;
    }
    std::fs::write(
        root.join("wrap.ts"),
        format!(
            "import * as entry from {};\nimport {{ wrap }} from {};\nexport default wrap(entry.default);\n",
            json!(extension.to_string_lossy()),
            json!(probe.to_string_lossy()),
        ),
    )?;
    let models = json!({"providers": {"fake": {
        "baseUrl": base_url,
        "apiKey": "sk-test",
        "api": "openai-completions",
        "models": [{"id": "fake-test", "contextWindow": 200000, "maxTokens": 8192}],
    }}});
    std::fs::write(root.join("agent/models.json"), models.to_string())?;
    Ok(())
}

fn reader<R: Read + Send + 'static>(mut source: R) -> JoinHandle<std::io::Result<String>> {
    std::thread::spawn(move || {
        let mut text = String::new();
        source.read_to_string(&mut text).map(|_| text)
    })
}

struct Run {
    status: Option<ExitStatus>,
    output: String,
    stderr: String,
    problems: Vec<String>,
}

fn exited(child: &mut Child, problems: &mut Vec<String>) -> Result<Option<ExitStatus>> {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() > deadline {
            child.kill()?;
            child.wait()?;
            problems.push(format!("still running {WAIT:?} after it was told to quit"));
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct State {
    is_streaming: bool,
    pending_message_count: usize,
}

struct Rpc {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Arc<Mutex<Vec<String>>>,
    stdout: JoinHandle<()>,
    stderr: JoinHandle<std::io::Result<String>>,
    next: usize,
}

impl Rpc {
    fn start(mut command: Command) -> Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = reader(child.stderr.take().ok_or("no stderr")?);
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        let stdout = std::thread::spawn(move || {
            for line in BufReader::new(stdout)
                .lines()
                .map_while(std::io::Result::ok)
            {
                if let Ok(mut lines) = sink.lock() {
                    lines.push(line);
                }
            }
        });
        Ok(Self {
            child,
            stdin: Some(stdin),
            lines,
            stdout,
            stderr,
            next: 0,
        })
    }

    fn send(&mut self, mut command: Value) -> Result<String> {
        self.next += 1;
        let id = format!("q{}", self.next);
        command["id"] = json!(id);
        let stdin = self.stdin.as_mut().ok_or("stdin is closed")?;
        writeln!(stdin, "{command}")?;
        stdin.flush()?;
        Ok(id)
    }

    fn answer(&self, id: &str) -> Result<Value> {
        let deadline = Instant::now() + WAIT;
        loop {
            let found = self
                .lines
                .lock()
                .map_err(|_| "stdout lock poisoned")?
                .iter()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|line| line["type"] == "response" && line["id"] == id);
            if let Some(found) = found {
                return Ok(found);
            }
            if Instant::now() > deadline {
                return Err(format!("no response to {id} within {WAIT:?}").into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn ask(&mut self, command: Value) -> Result<Value> {
        let id = self.send(command)?;
        self.answer(&id)
    }

    fn finish(mut self, mut problems: Vec<String>) -> Result<Run> {
        drop(self.stdin.take());
        let status = exited(&mut self.child, &mut problems)?;
        self.stdout.join().map_err(|_| "stdout reader panicked")?;
        let stderr = self.stderr.join().map_err(|_| "stderr reader panicked")??;
        let output = self
            .lines
            .lock()
            .map_err(|_| "stdout lock poisoned")?
            .join("\n");
        Ok(Run {
            status,
            output,
            stderr,
            problems,
        })
    }
}

fn succeeded(answer: &Value, problems: &mut Vec<String>) {
    if answer["success"] != true {
        problems.push(format!("answered {answer}"));
    }
}

fn rpc_steps(
    scenario: &Scenario,
    rpc: &mut Rpc,
    model: &Model,
    problems: &mut Vec<String>,
) -> Result<()> {
    let answer = rpc.ask(json!({"type": "prompt", "message": scenario.prompt}))?;
    succeeded(&answer, problems);
    if !scenario.queued.is_empty() {
        model.arrived(1)?;
        let mut sent = Vec::new();
        for queued in scenario.queued {
            sent.push(rpc.send(match queued {
                Queued::Steer(text) => json!({"type": "steer", "message": text}),
                Queued::FollowUp(text) => json!({"type": "follow_up", "message": text}),
            })?);
        }
        for id in sent {
            succeeded(&rpc.answer(&id)?, problems);
        }
        model.release()?;
    }
    settle(model, || {
        let answer = rpc.ask(json!({"type": "get_state"}))?;
        let state: State = serde_json::from_value(answer["data"].clone())
            .map_err(|error| format!("get_state answered {answer}: {error}"))?;
        Ok(!state.is_streaming && state.pending_message_count == 0)
    })
}

fn rpc_run(scenario: &Scenario, command: Command, model: &Model) -> Result<Run> {
    let mut rpc = Rpc::start(command)?;
    let mut problems = Vec::new();
    if let Err(error) = rpc_steps(scenario, &mut rpc, model, &mut problems) {
        problems.push(error.to_string());
    }
    model.release()?;
    rpc.finish(problems)
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
}

impl Tui {
    fn start(mut command: Command) -> Result<Self> {
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
            return Err(format!("openpty: {}", std::io::Error::last_os_error()).into());
        }
        let (master, slave) =
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        let input = File::from(master.try_clone()?);
        let mut source = File::from(master);
        let (stdin, stdout, stderr) = (slave.try_clone()?, slave.try_clone()?, slave);
        let child = command
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()?;
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&output);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(read) = source.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                if let Ok(mut sink) = sink.lock() {
                    sink.extend_from_slice(&chunk[..read]);
                }
            }
        });
        Ok(Self {
            child,
            input,
            output,
        })
    }

    fn screen(&self) -> Result<String> {
        self.since(0)
    }

    fn since(&self, from: usize) -> Result<String> {
        let output = self.output.lock().map_err(|_| "screen lock poisoned")?;
        Ok(plain(output.get(from..).unwrap_or_default()))
    }

    fn raw(&self) -> Result<bool> {
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(self.input.as_raw_fd(), &mut termios) } != 0 {
            return Err(format!("tcgetattr: {}", std::io::Error::last_os_error()).into());
        }
        Ok(termios.c_lflag & libc::ICANON == 0)
    }

    fn written(&self) -> Result<usize> {
        Ok(self
            .output
            .lock()
            .map_err(|_| "screen lock poisoned")?
            .len())
    }

    fn expect(&self, text: &str) -> Result<()> {
        let deadline = Instant::now() + WAIT;
        loop {
            if self.screen()?.contains(text) {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!("no {text:?} on the screen within {WAIT:?}").into());
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
        Ok(())
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

fn loaded(log: &str) -> bool {
    boundary(log).is_ok_and(|records| {
        records.iter().any(|record| {
            record["kind"] == "load"
                && matches!(
                    record["outcome"]["kind"].as_str(),
                    Some("return" | "resolve")
                )
        })
    })
}

fn tui_steps(scenario: &Scenario, tui: &mut Tui, model: &Model, log: &Path) -> Result<()> {
    let deadline = Instant::now() + WAIT;
    while !(std::fs::read_to_string(log).is_ok_and(|log| loaded(&log))
        && tui.raw()?
        && tui.screen()?.contains("fake-test"))
    {
        if Instant::now() > deadline {
            return Err(format!(
                "within {WAIT:?} the extension did not load, the terminal did not turn raw or the footer did not name fake-test"
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    submit(tui, model, scenario.prompt)?;
    if !scenario.queued.is_empty() {
        for queued in scenario.queued {
            let Queued::Steer(text) = queued else {
                return Err("a TUI scenario queues only steers".into());
            };
            tui.keys(&format!("{text}\r"))?;
            tui.expect(&format!("Steering: {text}"))?;
        }
        model.release()?;
    }
    settle(model, || Ok(true))
}

fn submit(tui: &mut Tui, model: &Model, prompt: &str) -> Result<()> {
    tui.keys(prompt)?;
    let deadline = Instant::now() + WAIT;
    loop {
        let from = tui.written()?;
        tui.keys("\r")?;
        while !tui.since(from)?.contains(STARTING) {
            if model.progress()?.served > 0 {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!("request 1 did not arrive within {WAIT:?}").into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn tui_run(scenario: &Scenario, command: Command, model: &Model, log: &Path) -> Result<Run> {
    let mut tui = Tui::start(command)?;
    let mut problems = Vec::new();
    if let Err(error) = tui_steps(scenario, &mut tui, model, log) {
        problems.push(error.to_string());
    }
    model.release()?;
    tui.keys("\x04")?;
    let status = exited(&mut tui.child, &mut problems)?;
    Ok(Run {
        status,
        output: tui.screen()?,
        stderr: String::new(),
        problems,
    })
}

fn canonical(value: Value, root: &str, docs: &Regex) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map
                .into_iter()
                .map(|(key, value)| (key, canonical(value, root, docs)))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().collect())
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| canonical(item, root, docs))
                .collect(),
        ),
        Value::String(text) => Value::String(
            docs.replace_all(&text.replace(root, "<root>"), "$1: <docs>")
                .into_owned(),
        ),
        Value::Number(number) => match number.as_f64() {
            Some(float) if float.fract() == 0.0 && float.abs() < 9_007_199_254_740_992.0 => {
                json!(float as i64)
            }
            _ => Value::Number(number),
        },
        other => other,
    }
}

fn volatile(value: &mut Value, ids: &mut HashMap<String, usize>) {
    match value {
        Value::Object(fields) => {
            let entry =
                fields.contains_key("parentId") || fields.get("type") == Some(&json!("session"));
            for (key, field) in fields.iter_mut() {
                let names_entry = (entry && key == "id")
                    || key == "parentId"
                    || key == "targetId"
                    || key.ends_with("EntryId");
                match field {
                    Value::String(id) if names_entry => {
                        let next = ids.len();
                        let order = *ids.entry(id.clone()).or_insert(next);
                        *field = json!(format!("<entry {order}>"));
                    }
                    Value::Number(_) | Value::String(_) if key == "timestamp" => {
                        *field = json!("<time>")
                    }
                    Value::Number(_) if key == "durationMs" => *field = json!("<duration>"),
                    _ => volatile(field, ids),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| volatile(item, ids)),
        _ => {}
    }
}

fn boundary(log: &str) -> Result<Vec<Value>> {
    let mut records: Vec<Value> = Vec::new();
    let mut at: HashMap<u64, usize> = HashMap::new();
    for (index, line) in log.lines().enumerate() {
        let mut record: Value = serde_json::from_str(line)
            .map_err(|error| format!("probe log line {}: {error}", index + 1))?;
        let id = record["id"]
            .as_u64()
            .ok_or_else(|| format!("probe log line {}: no id", index + 1))?;
        match record["kind"].as_str() {
            Some("return" | "resolve" | "throw" | "reject") => {
                let slot = at
                    .get(&id)
                    .ok_or_else(|| format!("probe log line {}: outcome of no call", index + 1))?;
                if let Value::Object(fields) = &mut record {
                    fields.remove("id");
                }
                records[*slot]["outcome"] = record;
            }
            _ => {
                at.insert(id, records.len());
                records.push(record);
            }
        }
    }
    records.sort_by_key(|record| record["id"].as_u64());
    for record in &mut records {
        if let Value::Object(fields) = record {
            fields.remove("id");
        }
    }
    Ok(records)
}

#[derive(Deserialize)]
struct Request {
    messages: Vec<RequestMessage>,
}

#[derive(Deserialize)]
struct RequestMessage {
    role: String,
    #[serde(default)]
    content: Option<Content>,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Parts(Vec<Part>),
}

#[derive(Deserialize)]
struct Part {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct ToolCall {
    function: Function,
}

#[derive(Deserialize, Serialize)]
struct Function {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Seen {
    System {
        role: String,
        #[serde(rename = "probeMark")]
        probe_mark: bool,
    },
    Said {
        role: String,
        text: String,
        #[serde(rename = "toolCalls", skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
}

impl RequestMessage {
    fn text(&self) -> String {
        match &self.content {
            None => String::new(),
            Some(Content::Text(text)) => text.clone(),
            Some(Content::Parts(parts)) => parts
                .iter()
                .filter_map(|part| part.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    fn seen(self) -> Seen {
        let text = self.text();
        if matches!(self.role.as_str(), "system" | "developer") {
            Seen::System {
                role: self.role,
                probe_mark: text.contains("PROBE-MARK"),
            }
        } else {
            Seen::Said {
                role: self.role,
                text,
                tool_calls: self.tool_calls,
            }
        }
    }
}

fn trace(log: &str, requests: &[Value], root: &Path, docs: &Regex) -> Result<Vec<Value>> {
    let root = root.to_string_lossy();
    let mut events = Vec::new();
    let mut ids = HashMap::new();
    for mut record in boundary(log)? {
        volatile(&mut record, &mut ids);
        events.push(canonical(record, &root, docs));
    }
    for (index, body) in requests.iter().enumerate() {
        let request: Request = serde_json::from_value(body.clone())
            .map_err(|error| format!("request {}: {error}", index + 1))?;
        let messages: Vec<Seen> = request
            .messages
            .into_iter()
            .map(RequestMessage::seen)
            .collect();
        events.push(canonical(
            json!({"request": index + 1, "messages": messages}),
            &root,
            docs,
        ));
    }
    Ok(events)
}

fn lines(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .flat_map(|event| {
            serde_json::to_string_pretty(event)
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn diff(old: &[String], new: &[String]) -> String {
    let mut common = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            common[i][j] = if old[i] == new[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let mut edits = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            edits.push((' ', &old[i]));
            i += 1;
            j += 1;
        } else if i < old.len() && (j == new.len() || common[i + 1][j] >= common[i][j + 1]) {
            edits.push(('-', &old[i]));
            i += 1;
        } else {
            edits.push(('+', &new[j]));
            j += 1;
        }
    }
    let near = |at: usize| {
        edits[at.saturating_sub(3)..(at + 4).min(edits.len())]
            .iter()
            .any(|(mark, _)| *mark != ' ')
    };
    let mut out = String::new();
    let mut skipped = false;
    for (at, (mark, line)) in edits.iter().enumerate() {
        if near(at) {
            if skipped {
                out.push_str("  ...\n");
            }
            skipped = false;
            out.push_str(&format!("{mark} {line}\n"));
        } else {
            skipped = true;
        }
    }
    out
}

fn side(
    scenario: &Scenario,
    bin: &Path,
    root: &Path,
    base_url: &str,
    model: &Model,
    files: (&Path, &Path),
) -> Result<(Run, Option<String>, Vec<Value>)> {
    let log = root.join("extension.log");
    prepare(root, base_url, files.0, files.1)?;
    model.reset(scenario, log.clone())?;
    let command = command(bin, root, scenario.mode);
    let run = match scenario.mode {
        Mode::Rpc => rpc_run(scenario, command, model)?,
        Mode::Tui => tui_run(scenario, command, model, &log)?,
    };
    let requests = model.requests()?;
    let output = match scenario.mode {
        Mode::Rpc => "stdout.jsonl",
        Mode::Tui => "screen.txt",
    };
    std::fs::write(root.join(output), &run.output)?;
    std::fs::write(root.join("stderr.txt"), &run.stderr)?;
    std::fs::write(
        root.join("requests.json"),
        serde_json::to_string_pretty(&requests)?,
    )?;
    Ok((run, std::fs::read_to_string(&log).ok(), requests))
}

fn main() -> Result<()> {
    let bins = [env_path("PI_UPSTREAM_BIN")?, env_path("PI_RS_BIN")?];
    let probe = env_path("PI_PARITY_PROBE")?;
    let extension = env_path("PI_QUEUE_EXTENSION")?;
    let only: Vec<String> = std::env::args().skip(1).collect();
    if let Some(unknown) = only.iter().find(|want| {
        !SCENARIOS
            .iter()
            .any(|scenario| scenario.name == want.as_str())
    }) {
        return Err(format!("no scenario named {unknown}").into());
    }
    let scenarios: Vec<&Scenario> = SCENARIOS
        .iter()
        .filter(|scenario| only.is_empty() || only.iter().any(|want| want == scenario.name))
        .collect();

    let model = Arc::new(Model::default());
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base_url = format!("http://127.0.0.1:{}/v1", listener.local_addr()?.port());
    let served = Arc::clone(&model);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let model = Arc::clone(&served);
            std::thread::spawn(move || {
                if let Err(error) = serve(stream, &model) {
                    eprintln!("fake model: {error}");
                }
            });
        }
    });

    let docs = Regex::new(r"(Main documentation|Additional docs|Examples): /\S+")?;
    let scratch =
        std::env::temp_dir().join(format!("extensions-queue-parity-{}", std::process::id()));
    let mut failed = Vec::new();
    for scenario in &scenarios {
        let name = scenario.name;
        let root = scratch.join(name).join("run");
        let mut found = Vec::new();
        let mut traces = Vec::new();
        for (side_name, bin) in SIDES.iter().zip(&bins) {
            let outcome = side(
                scenario,
                bin,
                &root,
                &base_url,
                &model,
                (&extension, &probe),
            );
            if root.exists() {
                std::fs::rename(&root, scratch.join(name).join(side_name))?;
            }
            let (run, log, requests) = match outcome {
                Ok(outcome) => outcome,
                Err(error) => {
                    found.push(format!("{side_name}: {error}"));
                    traces.push(Vec::new());
                    continue;
                }
            };
            found.extend(
                run.problems
                    .iter()
                    .map(|problem| format!("{side_name}: {problem}")),
            );
            if let Some(status) = run.status
                && !status.success()
            {
                found.push(format!("{side_name} exited {status}"));
            }
            if *side_name == SIDES[0] && !log.as_deref().is_some_and(loaded) {
                found.push(format!(
                    "{side_name} did not load the extension, so this scenario measures nothing"
                ));
            }
            if requests.len() != scenario.replies.len() {
                found.push(format!(
                    "{side_name} sent {} model requests, the scenario scripts {}",
                    requests.len(),
                    scenario.replies.len()
                ));
            }
            match trace(log.as_deref().unwrap_or_default(), &requests, &root, &docs) {
                Ok(trace) => traces.push(trace),
                Err(error) => {
                    found.push(format!("{side_name}: {error}"));
                    traces.push(Vec::new());
                }
            }
        }
        let differ = traces[0] != traces[1];
        if found.is_empty() && !differ {
            println!("{name}: same {} events", traces[0].len());
            continue;
        }
        println!("{name}: DIFFERS");
        for problem in &found {
            println!("  {}", problem.replace('\n', "\n    "));
        }
        if differ {
            println!("  events, - upstream + pi-rs:");
            print!("{}", diff(&lines(&traces[0]), &lines(&traces[1])));
        }
        failed.push(name);
    }
    println!(
        "extensions queue parity: {} of {} scenarios same",
        scenarios.len() - failed.len(),
        scenarios.len()
    );
    if !failed.is_empty() {
        return Err(format!(
            "{} of {} scenarios differ: {} (raw output kept under {})",
            failed.len(),
            scenarios.len(),
            failed.join(", "),
            scratch.display()
        )
        .into());
    }
    std::fs::remove_dir_all(&scratch)?;
    Ok(())
}
