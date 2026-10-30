use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SIDES: [&str; 2] = ["upstream", "pi-rs"];

const COMPARED_EVENTS: [&str; 11] = [
    "agent_start",
    "turn_start",
    "message_end",
    "tool_execution_start",
    "tool_execution_update",
    "tool_execution_end",
    "turn_end",
    "agent_end",
    "response",
    "extension_ui_request",
    "extension_error",
];

const DIALOGS: [&str; 4] = ["select", "confirm", "input", "editor"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    #[serde(default)]
    prompts: Vec<String>,
    #[serde(default)]
    rpc: Vec<Value>,
    turns: Vec<Turn>,
    #[serde(default)]
    files: BTreeMap<String, String>,
    #[serde(default)]
    agent_files: BTreeMap<String, String>,
    #[serde(default)]
    setup: Vec<Vec<String>>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    providers: Map<String, Value>,
}

impl Default for Fixture {
    fn default() -> Self {
        Fixture {
            files: BTreeMap::new(),
            agent_files: BTreeMap::new(),
            setup: Vec::new(),
            args: Vec::new(),
            providers: Map::new(),
            prompts: vec!["hi".into()],
            rpc: Vec::new(),
            turns: vec![Turn {
                text: Some("done".into()),
                tool_calls: Vec::new(),
                usage: None,
                after: None,
            }],
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Exemption {
    examples: Vec<String>,
    handlers: Vec<String>,
    reason: String,
    #[serde(default)]
    ran: bool,
}

struct Exempt {
    examples: Vec<(Regex, bool)>,
    handlers: Vec<(Regex, bool)>,
    entry: Exemption,
}

impl Exempt {
    fn new(entry: Exemption) -> Result<Self> {
        let globs = |patterns: &[String]| -> Result<Vec<(Regex, bool)>> {
            patterns
                .iter()
                .map(|pattern| Ok((glob(pattern)?, false)))
                .collect()
        };
        Ok(Exempt {
            examples: globs(&entry.examples)?,
            handlers: globs(&entry.handlers)?,
            entry,
        })
    }

    fn covers(&mut self, example: &str, handler: &str) -> bool {
        let example = self
            .examples
            .iter()
            .position(|(glob, _)| glob.is_match(example));
        let pattern = self
            .handlers
            .iter()
            .position(|(glob, _)| glob.is_match(handler));
        let (Some(example), Some(pattern)) = (example, pattern) else {
            return false;
        };
        self.examples[example].1 = true;
        self.handlers[pattern].1 = true;
        true
    }

    fn stale(&self) -> Vec<String> {
        self.examples
            .iter()
            .chain(&self.handlers)
            .zip(self.entry.examples.iter().chain(&self.entry.handlers))
            .filter(|((_, hit), _)| !hit)
            .map(|(_, pattern)| pattern.clone())
            .collect()
    }
}

fn glob(pattern: &str) -> Result<Regex> {
    Ok(Regex::new(&format!(
        "^{}$",
        regex::escape(pattern).replace(r"\*", ".*")
    ))?)
}

#[derive(Default, PartialEq)]
struct Coverage {
    registered: BTreeSet<String>,
    ran: BTreeSet<String>,
    finished: BTreeSet<String>,
}

fn handlers(path: &str, value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::String(text) if text == "<function>" => {
            out.insert(path.to_owned());
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                handlers(&format!("{path}[{index}]"), item, out);
            }
        }
        Value::Object(fields) if !fields.keys().any(|key| key.starts_with('<')) => {
            for (key, field) in fields {
                handlers(&format!("{path}.{key}"), field, out);
            }
        }
        _ => {}
    }
}

#[derive(Deserialize)]
struct Record {
    #[serde(default)]
    id: Option<u64>,
    kind: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    args: Vec<Value>,
    #[serde(default)]
    timer: Option<String>,
}

fn coverage(log: &str) -> Result<Coverage> {
    let mut found = Coverage::default();
    let mut calls: HashMap<u64, String> = HashMap::new();
    for (index, line) in log.lines().enumerate() {
        let record: Record = serde_json::from_str(line)
            .map_err(|error| format!("probe log line {}: {error}", index + 1))?;
        if record.timer.as_deref() == Some(record.path.as_str()) {
            continue;
        }
        let path = record.path;
        match record.kind.as_str() {
            "callback" => {
                if let Some(id) = record.id {
                    calls.insert(id, path.clone());
                }
                found.ran.insert(path);
            }
            "return" | "resolve" => {
                if let Some(path) = record.id.and_then(|id| calls.get(&id)) {
                    found.finished.insert(path.clone());
                }
            }
            "call" if path.starts_with("pi.") && !path.contains('#') => {
                for (index, arg) in record.args.iter().enumerate() {
                    handlers(&format!("{path}#{index}"), arg, &mut found.registered);
                }
                if let Some(flag) = path.strip_prefix("pi.getFlag") {
                    found.ran.insert(format!("pi.registerFlag{flag}"));
                    found.finished.insert(format!("pi.registerFlag{flag}"));
                } else if path.starts_with("pi.registerFlag(") {
                    found.registered.insert(path);
                }
            }
            _ => {}
        }
    }
    Ok(found)
}

struct Case {
    name: String,
    entry: PathBuf,
    fixture: Fixture,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Turn {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    tool_calls: Vec<Call>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    after: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Usage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    name: String,
    arguments: Value,
    #[serde(default)]
    fails: bool,
}

struct Run {
    status: ExitStatus,
    stdout: String,
    stderr: String,
    log: Option<String>,
    files: BTreeMap<String, String>,
}

fn seeded(dir: &Path, path: &str) -> Result<PathBuf> {
    let relative = Path::new(path);
    if !relative
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)))
    {
        return Err(format!("seeded file {path} is not a plain relative path").into());
    }
    Ok(dir.join(relative))
}

fn snapshot(dir: &Path, prefix: &str, out: &mut BTreeMap<String, String>) -> Result<()> {
    for item in std::fs::read_dir(dir)? {
        let path = item?.path();
        let name = format!(
            "{prefix}{}",
            path.file_name().ok_or("unnamed file")?.to_string_lossy()
        );
        if name == ".git" {
            continue;
        }
        if path.is_dir() {
            snapshot(&path, &format!("{name}/"), out)?;
        } else {
            out.insert(
                name,
                String::from_utf8_lossy(&std::fs::read(&path)?).into_owned(),
            );
        }
    }
    Ok(())
}

type Script = Arc<Mutex<HashMap<String, (Vec<Turn>, Vec<Value>, PathBuf)>>>;

fn fired(log: &Path, path: &str) -> bool {
    std::fs::read_to_string(log).is_ok_and(|log| {
        log.lines()
            .filter_map(|line| serde_json::from_str::<Record>(line).ok())
            .any(|record| record.kind == "callback" && record.path == path)
    })
}

fn env_path(name: &str) -> Result<PathBuf> {
    match std::env::var_os(name) {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Err(format!("set {name}").into()),
    }
}

fn env_dir(name: &str) -> Result<PathBuf> {
    let dir = env_path(name)?;
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(format!("{name}: {} is not a directory", dir.display()).into())
    }
}

fn call_id(turn: usize, call: usize) -> String {
    format!("call_{turn}_{call}")
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

fn stream_body(turn_index: usize, turn: &Turn) -> String {
    let id = format!("c{turn_index}");
    let mut delta = Map::new();
    delta.insert("role".into(), json!("assistant"));
    if let Some(text) = &turn.text {
        delta.insert("content".into(), json!(text));
    }
    if !turn.tool_calls.is_empty() {
        let calls: Vec<Value> = turn
            .tool_calls
            .iter()
            .enumerate()
            .map(|(index, call)| {
                json!({"index": index, "id": call_id(turn_index, index), "type": "function",
                    "function": {"name": call.name, "arguments": call.arguments.to_string()}})
            })
            .collect();
        delta.insert("tool_calls".into(), Value::Array(calls));
    }
    let finish = if turn.tool_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };
    let mut last =
        json!({"id": id, "choices": [{"index": 0, "delta": {}, "finish_reason": finish}]});
    if let Some(usage) = &turn.usage {
        last["usage"] = json!({"prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.prompt_tokens + usage.completion_tokens});
    }
    [
        json!({"id": id, "choices": [{"index": 0, "delta": delta, "finish_reason": null}]}),
        last,
    ]
    .iter()
    .map(|chunk| format!("data: {chunk}\n\n"))
    .chain(["data: [DONE]\n\n".to_owned()])
    .collect()
}

fn serve(mut stream: TcpStream, script: &Script) -> Result<()> {
    let (path, body) = read_request(&mut stream)?;
    let label = path
        .trim_start_matches('/')
        .strip_suffix("/v1/chat/completions")
        .ok_or_else(|| format!("unexpected request path {path}"))?
        .to_owned();
    let (next, after, log) = {
        let mut script = script.lock().map_err(|_| "script lock poisoned")?;
        let (turns, requests, log) = script
            .get_mut(&label)
            .ok_or_else(|| format!("request for unknown run {label}"))?;
        requests.push(body);
        let index = requests.len() - 1;
        let turn = turns.get(index);
        (
            turn.map(|turn| stream_body(index, turn)),
            turn.and_then(|turn| turn.after.clone()),
            log.clone(),
        )
    };
    if let Some(after) = after {
        let deadline = Instant::now() + Duration::from_secs(100);
        while !fired(&log, &after) {
            if Instant::now() > deadline {
                return Err(
                    format!("{after} never ran, so the turn waiting on it was not sent").into(),
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
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

fn run(
    (bin, package): &(PathBuf, PathBuf),
    root: &Path,
    base_url: &str,
    case: &Case,
    probe: &Path,
) -> Result<Run> {
    for dir in ["home", "agent", "cwd"] {
        std::fs::create_dir_all(root.join(dir))?;
    }
    std::os::unix::fs::symlink(package, root.join("pkg"))?;
    let extension = root.join("wrap.ts");
    std::fs::write(
        &extension,
        format!(
            "import * as entry from {};\nimport {{ wrap }} from {};\nexport default wrap(entry.default);\n",
            json!(case.entry.to_string_lossy()),
            json!(probe.to_string_lossy()),
        ),
    )?;
    for (dir, files) in [
        ("cwd", &case.fixture.files),
        ("agent", &case.fixture.agent_files),
    ] {
        for (path, content) in files {
            let target = seeded(&root.join(dir), path)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, content)?;
        }
    }
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        [probe.parent().ok_or("probe without a parent")?.join("bin")]
            .into_iter()
            .chain(std::env::split_paths(&inherited_path)),
    )?;
    let environment = |command: &mut Command| {
        command
            .current_dir(root.join("cwd"))
            .env_clear()
            .env("HOME", root.join("home"))
            .env("PATH", &path)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .envs(
                ["GIT_AUTHOR_DATE", "GIT_COMMITTER_DATE"].map(|key| (key, "2000-01-01T00:00:00Z")),
            );
    };
    for command in &case.fixture.setup {
        let (bin, args) = command.split_first().ok_or("empty fixture setup command")?;
        let mut setup = Command::new(bin);
        environment(&mut setup);
        let output = setup.args(args).output()?;
        if !output.status.success() {
            return Err(format!(
                "fixture setup {command:?}: {}: {}{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
    }
    let mut providers = case.fixture.providers.clone();
    providers.insert(
        "fake".into(),
        json!({
            "api": "openai-completions",
            "models": [{"id": "fake-test", "contextWindow": 200000, "maxTokens": 8192}],
        }),
    );
    for provider in providers.values_mut() {
        let fields = provider
            .as_object_mut()
            .ok_or("a fixture provider is not an object")?;
        fields.insert("baseUrl".into(), json!(base_url));
        fields.insert("apiKey".into(), json!("sk-test"));
    }
    std::fs::write(
        root.join("agent/models.json"),
        json!({ "providers": providers }).to_string(),
    )?;
    let rpc = !case.fixture.rpc.is_empty();
    let mut command = Command::new(bin);
    environment(&mut command);
    let mut child = command
        .args([
            "--provider",
            "fake",
            "--model",
            "fake-test",
            "--no-session",
            "--mode",
        ])
        .args(if rpc {
            &["rpc"][..]
        } else {
            &["json", "-p"][..]
        })
        .args(&case.fixture.args)
        .arg("-e")
        .arg(&extension)
        .args(&case.fixture.prompts)
        .env("PI_CODING_AGENT_DIR", root.join("agent"))
        .env("PI_PACKAGE_DIR", root.join("pkg"))
        .env("PI_OFFLINE", "1")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_TELEMETRY", "0")
        .env("PI_PARITY_LOG", root.join("extension.log"))
        .envs(std::env::var_os("PI_EXTENSION_HOST").map(|host| ("PI_EXTENSION_HOST", host)))
        .stdin(if rpc { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let (sender, lines) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line?).is_err() {
                break;
            }
        }
        std::io::Result::Ok(())
    });
    let stderr = std::thread::spawn(move || {
        let mut text = String::new();
        stderr.read_to_string(&mut text).map(|_| text)
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut stdout = String::new();
    if let Some(stdin) = child.stdin.take()
        && let Err(error) = drive(stdin, &lines, &mut stdout, &case.fixture.rpc, deadline)
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("{}: {error}", bin.display()).into());
    }
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{} did not finish within 120s", bin.display()).into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    reader.join().map_err(|_| "stdout reader panicked")??;
    for line in lines.try_iter() {
        stdout.push_str(&line);
        stdout.push('\n');
    }
    let stderr = stderr.join().map_err(|_| "stderr reader panicked")??;
    std::fs::write(root.join("stdout.jsonl"), &stdout)?;
    std::fs::write(root.join("stderr.txt"), &stderr)?;
    let mut files = BTreeMap::new();
    snapshot(&root.join("cwd"), "", &mut files)?;
    Ok(Run {
        status,
        stdout,
        stderr,
        log: std::fs::read_to_string(root.join("extension.log")).ok(),
        files,
    })
}

fn drive(
    mut stdin: ChildStdin,
    lines: &Receiver<String>,
    out: &mut String,
    commands: &[Value],
    deadline: Instant,
) -> Result<()> {
    for (index, command) in commands.iter().enumerate() {
        let id = format!("rpc-{index}");
        let mut command = command.clone();
        command["id"] = json!(id);
        writeln!(stdin, "{command}")?;
        let mut settling = false;
        loop {
            let line = match lines.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("no answer to {command} within 120s").into());
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!("exited before it answered {command}").into());
                }
            };
            out.push_str(&line);
            out.push('\n');
            let event: Value = serde_json::from_str(&line).unwrap_or_default();
            if event["type"] == "extension_ui_request"
                && event["method"]
                    .as_str()
                    .is_some_and(|method| DIALOGS.contains(&method))
            {
                let answer =
                    json!({"type": "extension_ui_response", "id": event["id"], "cancelled": true});
                writeln!(stdin, "{answer}")?;
            } else if settling {
                if event["type"] == "agent_settled" {
                    break;
                }
            } else if event["type"] == "response" && event["id"] == id.as_str() {
                if command["type"] == "prompt" && event["data"]["disposition"] == "started" {
                    settling = true;
                } else {
                    break;
                }
            }
        }
    }
    Ok(())
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

fn message_metadata(message: &mut Value) {
    if let Value::Object(message) = message {
        message.remove("timestamp");
        message.remove("durationMs");
        if message.get("role") == Some(&json!("assistant")) {
            message.remove("thinkingLevel");
        }
        if let Some(Value::Array(tools)) = message.get_mut("toolsAdded") {
            for tool in tools.iter_mut().filter_map(Value::as_object_mut) {
                tool.remove("constrainedSampling");
            }
        }
    }
}

fn order(id: &str, ids: &mut HashMap<String, usize>) -> Value {
    let next = ids.len();
    json!(format!(
        "<entry {}>",
        ids.entry(id.to_owned()).or_insert(next)
    ))
}

fn volatile(value: &mut Value, ids: &mut HashMap<String, usize>) {
    match value {
        Value::Object(fields) => {
            let entry = fields.contains_key("parentId")
                || fields.get("type") == Some(&json!("session"))
                || fields.get("type") == Some(&json!("extension_ui_request"));
            for (key, field) in fields.iter_mut() {
                let names_entry = (entry && key == "id")
                    || key == "parentId"
                    || key == "targetId"
                    || key == "entryId"
                    || key == "sessionId"
                    || key.ends_with("EntryId")
                    || key.ends_with("LeafId");
                match field {
                    Value::String(id) if names_entry => *field = order(id, ids),
                    Value::Number(_) | Value::String(_) if key == "timestamp" => {
                        *field = json!("<time>")
                    }
                    Value::Number(_) if key == "durationMs" => *field = json!("<duration>"),
                    Value::Number(_) if key == "wall_time_seconds" => *field = json!("<seconds>"),
                    _ => volatile(field, ids),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| volatile(item, ids)),
        _ => {}
    }
}

fn named(value: &mut Value, ids: &HashMap<String, usize>) {
    match value {
        Value::String(text) => {
            for (id, order) in ids {
                if text.contains(id.as_str()) {
                    *text = text.replace(id.as_str(), &format!("<entry {order}>"));
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| named(item, ids)),
        Value::Object(fields) => fields.values_mut().for_each(|field| named(field, ids)),
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

fn loaded(run: &Run) -> bool {
    run.log.as_deref().is_some_and(|log| {
        boundary(log).is_ok_and(|records| {
            records.iter().any(|record| {
                record["kind"] == "load"
                    && matches!(
                        record["outcome"]["kind"].as_str(),
                        Some("return" | "resolve")
                    )
            })
        })
    })
}

fn extension_tools(log: &str) -> BTreeSet<String> {
    log.lines()
        .filter_map(|line| serde_json::from_str::<Record>(line).ok())
        .filter(|record| record.kind == "call")
        .filter_map(|record| {
            let (name, rest) = record
                .path
                .strip_prefix("pi.registerTool(")?
                .rsplit_once(')')?;
            (rest.is_empty() || rest.starts_with('@')).then(|| name.to_owned())
        })
        .collect()
}

fn trace(run: &Run, root: &Path, docs: &Regex) -> Result<Vec<Value>> {
    let root = root.to_string_lossy();
    let tools = extension_tools(run.log.as_deref().unwrap_or_default());
    let mut events = Vec::new();
    for (index, line) in run.stdout.lines().enumerate() {
        let mut event: Value = serde_json::from_str(line)
            .map_err(|error| format!("stdout line {}: {error}", index + 1))?;
        let kind = event["type"]
            .as_str()
            .ok_or_else(|| format!("stdout line {}: event has no type", index + 1))?
            .to_owned();
        if !COMPARED_EVENTS.contains(&kind.as_str()) {
            continue;
        }
        match kind.as_str() {
            "message_end" | "turn_end" => {
                message_metadata(&mut event["message"]);
                if let Some(Value::Array(messages)) = event.get_mut("toolResults") {
                    messages.iter_mut().for_each(message_metadata);
                }
            }
            "agent_end" => {
                if let Some(Value::Array(messages)) = event.get_mut("messages") {
                    messages.iter_mut().for_each(message_metadata);
                }
            }
            "tool_execution_end" => {
                if let Value::Object(fields) = &mut event {
                    fields.remove("durationMs");
                }
            }
            "tool_execution_update"
                if event["toolName"]
                    .as_str()
                    .is_some_and(|name| tools.contains(name)) =>
            {
                if let Some(Value::Object(partial)) = event.get_mut("partialResult") {
                    partial.remove("details");
                }
            }
            _ => {}
        }
        events.push(event);
    }
    let mut records = boundary(run.log.as_deref().unwrap_or_default())?;
    records.retain(|record| record["tick"].as_u64().is_none_or(|tick| tick == 1));
    events.extend(records);
    let mut ids = HashMap::new();
    events
        .iter_mut()
        .for_each(|event| volatile(event, &mut ids));
    let mut events: Vec<Value> = events
        .into_iter()
        .map(|mut event| {
            named(&mut event, &ids);
            canonical(event, &root, docs)
        })
        .collect();
    if !run.stderr.trim().is_empty() {
        events.push(canonical(
            json!({"stderr": run.stderr.lines().collect::<Vec<_>>()}),
            &root,
            docs,
        ));
    }
    if !run.files.is_empty() {
        events.push(canonical(json!({"files": run.files}), &root, docs));
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

fn written(value: &Value) -> Value {
    let dropped = |value: &Value| matches!(value.as_str(), Some("<undefined>" | "<function>"));
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, field)| !key.starts_with('<') && !dropped(field))
                .map(|(key, field)| (key.clone(), written(field)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| {
                    if dropped(item) {
                        Value::Null
                    } else {
                        written(item)
                    }
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn progress(side: &str, run: &Run) -> Result<Vec<String>> {
    let update = Regex::new(r"^pi\.registerTool\((.+)\)(@\d+)?#0\.execute#3$")?;
    let mut given: HashMap<String, Vec<Value>> = HashMap::new();
    for record in run
        .log
        .as_deref()
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Record>(line).ok())
        .filter(|record| record.kind == "call")
    {
        if let (Some(name), Some(partial)) = (update.captures(&record.path), record.args.first()) {
            given
                .entry(name[1].to_owned())
                .or_default()
                .push(written(partial));
        }
    }
    let tools = extension_tools(run.log.as_deref().unwrap_or_default());
    let mut from: HashMap<String, usize> = HashMap::new();
    let mut found = Vec::new();
    for (index, event) in run
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["type"] == "tool_execution_update")
        .filter(|event| {
            event["toolName"]
                .as_str()
                .is_some_and(|name| tools.contains(name))
        })
        .enumerate()
    {
        let name = event["toolName"].as_str().unwrap_or_default().to_owned();
        let calls = given.get(&name).map_or(&[][..], Vec::as_slice);
        let start = from.get(&name).copied().unwrap_or_default();
        let partial = &event["partialResult"];
        match (start..calls.len()).find(|&at| {
            let mut keys: BTreeSet<&String> = BTreeSet::new();
            for value in [partial, &calls[at]] {
                keys.extend(value.as_object().into_iter().flat_map(Map::keys));
            }
            keys.iter().all(|key| {
                calls[at..]
                    .iter()
                    .any(|call| call.get(key.as_str()) == partial.get(key.as_str()))
            }) && (partial.is_object() || calls[at..].contains(partial))
        }) {
            Some(at) => {
                from.insert(name, at);
            }
            None => found.push(format!(
                "{side} wrote tool_execution_update {} for {name} with a partialResult its execute did not pass to onUpdate: {}",
                index + 1,
                event["partialResult"]
            )),
        }
    }
    Ok(found)
}

fn exemption(exempts: &mut [Exempt], example: &str, handler: &str) -> Option<(bool, String)> {
    exempts.iter_mut().find_map(|exempt| {
        exempt
            .covers(example, handler)
            .then(|| (exempt.entry.ran, exempt.entry.reason.clone()))
    })
}

fn problems(side: &str, run: &Run, requests: usize, fixture: &Fixture) -> Vec<String> {
    let mut found = Vec::new();
    match progress(side, run) {
        Ok(problems) => found.extend(problems),
        Err(error) => found.push(format!("{side}: {error}")),
    }
    if !run.status.success() {
        found.push(format!("{side} exited {}", run.status));
    }
    if side == SIDES[0] && !loaded(run) {
        found.push(format!(
            "{side} did not load the entry, so this case measures nothing"
        ));
    }
    if requests != fixture.turns.len() {
        found.push(format!(
            "{side} sent {requests} model requests, the fixture scripts {}",
            fixture.turns.len()
        ));
    }
    let ends: Vec<Value> = run
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["type"] == "tool_execution_end")
        .collect();
    for (turn_index, turn) in fixture.turns.iter().enumerate() {
        for (index, call) in turn.tool_calls.iter().enumerate() {
            let id = call_id(turn_index, index);
            match ends.iter().find(|event| event["toolCallId"] == id.as_str()) {
                Some(end) if end["isError"] == call.fails => {}
                Some(end) if call.fails => found.push(format!(
                    "{side} ran {} ({id}), the fixture expects an error: {}",
                    call.name, end["result"]
                )),
                Some(end) => found.push(format!(
                    "{side} never ran {} ({id}): {}",
                    call.name, end["result"]
                )),
                None => found.push(format!(
                    "{side} never ran {} ({id}): no tool_execution_end",
                    call.name
                )),
            }
        }
    }
    found
}

fn main() -> Result<()> {
    let programs = [
        (
            env_path("PI_UPSTREAM_BIN")?,
            env_dir("PI_UPSTREAM_PACKAGE_DIR")?,
        ),
        (env_path("PI_RS_BIN")?, env_dir("PI_RS_PACKAGE_DIR")?),
    ];
    let fixtures_dir = env_path("PI_PARITY_FIXTURES")?;
    let probe = fixtures_dir.join("probe.ts");
    let only: Vec<String> = std::env::args().skip(1).collect();
    let mut entries = Vec::new();
    for item in std::fs::read_dir(fixtures_dir.join("upstream"))? {
        let path = item?.path();
        let name = path
            .file_stem()
            .ok_or("entry without a name")?
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            let index = path.join("index.ts");
            if !index.is_file() {
                return Err(
                    format!("{}: a directory entry without index.ts", path.display()).into(),
                );
            }
            entries.push((name, index));
        } else if path.extension().is_some_and(|ext| ext == "ts") {
            entries.push((name, path));
        }
    }
    if entries.is_empty() {
        return Err(format!("no entries in {}/upstream", fixtures_dir.display()).into());
    }
    let exempt_path = fixtures_dir.join("unexercised.json");
    let mut exempts = Vec::new();
    for entry in serde_json::from_str::<Vec<Exemption>>(&std::fs::read_to_string(&exempt_path)?)
        .map_err(|error| format!("{}: {error}", exempt_path.display()))?
    {
        exempts.push(Exempt::new(entry)?);
    }
    let mut scenarios: HashMap<String, Vec<(String, Fixture)>> = HashMap::new();
    for item in std::fs::read_dir(&fixtures_dir)? {
        let path = item?.path();
        if path.extension().is_none_or(|ext| ext != "json") || path == exempt_path {
            continue;
        }
        let fixture: Fixture = serde_json::from_str(&std::fs::read_to_string(&path)?)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if fixture.prompts.is_empty() == fixture.rpc.is_empty() {
            return Err(format!(
                "{}: holds prompts or rpc commands, not both or neither",
                path.display()
            )
            .into());
        }
        if let Some(command) = fixture
            .rpc
            .iter()
            .find(|command| !command["type"].is_string() || command.get("id").is_some())
        {
            return Err(format!(
                "{}: rpc command {command} needs a type and no id",
                path.display()
            )
            .into());
        }
        if fixture.agent_files.contains_key("models.json") {
            return Err(format!(
                "{}: agent_files holds models.json, which the check writes",
                path.display()
            )
            .into());
        }
        if fixture.providers.contains_key("fake") {
            return Err(format!(
                "{}: providers holds fake, the provider the check serves",
                path.display()
            )
            .into());
        }
        let name = path
            .file_stem()
            .ok_or("scenario without a name")?
            .to_string_lossy()
            .into_owned();
        let entry = name
            .split_once('.')
            .map_or(name.as_str(), |(entry, _)| entry);
        if !entries.iter().any(|(known, _)| known == entry) {
            return Err(format!("{}: names no entry in upstream/", path.display()).into());
        }
        scenarios
            .entry(entry.to_owned())
            .or_default()
            .push((name, fixture));
    }
    let mut groups: Vec<(String, bool, Vec<Case>)> = Vec::new();
    for (entry, path) in &entries {
        let mut cases = scenarios
            .remove(entry)
            .unwrap_or_else(|| vec![(entry.clone(), Fixture::default())]);
        cases.sort_by(|a, b| a.0.cmp(&b.0));
        let whole = only.is_empty() || only.contains(entry);
        let cases: Vec<Case> = cases
            .into_iter()
            .filter(|(name, _)| whole || only.contains(name))
            .map(|(name, fixture)| Case {
                name,
                entry: path.clone(),
                fixture,
            })
            .collect();
        if !cases.is_empty() {
            groups.push((entry.clone(), whole, cases));
        }
    }
    if groups.is_empty() {
        return Err(format!("no case matches {}", only.join(" ")).into());
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    let total: usize = groups.iter().map(|(_, _, cases)| cases.len()).sum();

    let script: Script = Arc::new(Mutex::new(HashMap::new()));
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let served = Arc::clone(&script);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let script = Arc::clone(&served);
            std::thread::spawn(move || {
                if let Err(error) = serve(stream, &script) {
                    eprintln!("fake model: {error}");
                }
            });
        }
    });

    let docs = Regex::new(r"(Main documentation|Additional docs|Examples): /\S+")?;
    let scratch = std::env::temp_dir().join(format!("extensions-parity-{}", std::process::id()));
    let mut failed = Vec::new();
    let mut unrun = 0;
    let mut unfinished = 0;
    for (entry, whole, cases) in &groups {
        let mut outcomes = Vec::new();
        for case in cases {
            let (name, fixture) = (&case.name, &case.fixture);
            let mut found = Vec::new();
            let mut traces = Vec::new();
            let mut covered = Vec::new();
            // Both sides run at one path and one URL, so what derives from them (the prompt's cwd,
            // its length, the tokens it counts) matches; each run then moves to its side's name.
            let root = scratch.join(name).join("run");
            let base_url = format!("http://127.0.0.1:{port}/{name}/v1");
            for (side, program) in SIDES.iter().zip(&programs) {
                script.lock().map_err(|_| "script lock poisoned")?.insert(
                    name.clone(),
                    (
                        fixture.turns.clone(),
                        Vec::new(),
                        root.join("extension.log"),
                    ),
                );
                let outcome = run(program, &root, &base_url, case, &probe);
                if root.exists() {
                    std::fs::rename(&root, scratch.join(name).join(side))?;
                }
                match outcome {
                    Ok(run) => {
                        let requests = script
                            .lock()
                            .map_err(|_| "script lock poisoned")?
                            .get(name)
                            .map_or(0, |(_, requests, _)| requests.len());
                        found.extend(problems(side, &run, requests, fixture));
                        match coverage(run.log.as_deref().unwrap_or_default()) {
                            Ok(coverage) => covered.push(coverage),
                            Err(error) => {
                                found.push(format!("{side}: {error}"));
                                covered.push(Coverage::default());
                            }
                        }
                        match trace(&run, &root, &docs) {
                            Ok(trace) => traces.push(trace),
                            Err(error) => {
                                found.push(format!("{side}: {error}"));
                                traces.push(Vec::new());
                            }
                        }
                    }
                    Err(error) => {
                        found.push(format!("{side}: {error}"));
                        traces.push(Vec::new());
                        covered.push(Coverage::default());
                    }
                }
            }
            outcomes.push((found, traces, covered));
        }
        let ran: BTreeSet<String> = outcomes
            .iter()
            .flat_map(|(_, _, covered)| covered[0].ran.iter().cloned())
            .collect();
        let finished: BTreeSet<String> = outcomes
            .iter()
            .flat_map(|(_, _, covered)| covered[0].finished.iter().cloned())
            .collect();
        for (case, (mut found, traces, covered)) in cases.iter().zip(outcomes) {
            let name = &case.name;
            let mut report = Vec::new();
            for (side, coverage) in SIDES.iter().zip(&covered) {
                for (kind, handlers) in [
                    ("registered", &coverage.registered),
                    ("ran", &coverage.ran),
                    ("finished", &coverage.finished),
                ] {
                    let handlers: Vec<_> = handlers.iter().map(String::as_str).collect();
                    report.push(format!(
                        "{side} {kind}: {}",
                        if handlers.is_empty() {
                            "nothing".to_owned()
                        } else {
                            handlers.join(", ")
                        }
                    ));
                }
            }
            let upstream = &covered[0];
            if *whole {
                let mut elsewhere = Vec::new();
                let mut skipped = Vec::new();
                let mut partial = Vec::new();
                for handler in &upstream.registered {
                    let ran_anywhere = ran.contains(handler);
                    if ran_anywhere && !upstream.ran.contains(handler) {
                        elsewhere.push(handler.as_str());
                    }
                    match (ran_anywhere, exemption(&mut exempts, entry, handler)) {
                        (true, Some((true, _))) => partial.push(handler.as_str()),
                        (true, None) if !finished.contains(handler) => found.push(format!(
                            "upstream ran {handler}, but no call in any scenario of {entry} returned: drive it to completion or name it in unexercised.json as ran"
                        )),
                        (true, Some((false, reason))) => found.push(format!(
                            "upstream ran {handler}, which unexercised.json says cannot run: {reason}"
                        )),
                        (false, Some((true, reason))) => found.push(format!(
                            "upstream never ran {handler}, which unexercised.json says runs but cannot finish: {reason}"
                        )),
                        (false, Some((false, _))) => skipped.push(handler.as_str()),
                        (false, None) => found.push(format!(
                            "upstream never ran {handler} in any scenario of {entry}: drive it from a fixture or name it in unexercised.json"
                        )),
                        (true, None) => {}
                    }
                }
                for (label, handlers) in [
                    ("ran in another scenario", &elsewhere),
                    ("not run, exempt", &skipped),
                    ("ran, cannot finish", &partial),
                ] {
                    if !handlers.is_empty() {
                        report.push(format!("{label}: {}", handlers.join(", ")));
                    }
                }
                unrun += skipped.len();
                unfinished += partial.len();
            } else {
                report.push(format!(
                    "coverage not judged: run {entry} to judge it across all its scenarios"
                ));
            }
            if covered[1] != covered[0] {
                found.push("registered or invoked handlers differ between sides".to_owned());
                let missed: Vec<_> = upstream.ran.difference(&covered[1].ran).cloned().collect();
                let extra: Vec<_> = covered[1].ran.difference(&upstream.ran).cloned().collect();
                if !missed.is_empty() {
                    report.push(format!("{} never ran: {}", SIDES[1], missed.join(", ")));
                }
                if !extra.is_empty() {
                    report.push(format!("{} alone ran: {}", SIDES[1], extra.join(", ")));
                }
            }
            let differ = traces[0] != traces[1];
            if found.is_empty() && !differ {
                println!("{name}: same {} events", traces[0].len());
                for line in &report {
                    println!("  {line}");
                }
                continue;
            }
            println!("{name}: DIFFERS");
            for line in &report {
                println!("  {line}");
            }
            for problem in &found {
                println!("  {}", problem.replace('\n', "\n    "));
            }
            if differ {
                println!("  events, - upstream + pi-rs:");
                print!("{}", diff(&lines(&traces[0]), &lines(&traces[1])));
            }
            failed.push(name.clone());
        }
    }
    println!(
        "extensions parity: {} of {total} cases same",
        total - failed.len()
    );
    println!(
        "unexercised.json: {unrun} handlers no scenario runs and {unfinished} that run but cannot finish"
    );
    let stale: Vec<String> = exempts
        .iter()
        .filter(|_| only.is_empty())
        .flat_map(Exempt::stale)
        .collect();
    if !stale.is_empty() {
        return Err(format!(
            "unexercised.json names patterns that match nothing left unrun or unfinished: {}",
            stale.join(", ")
        )
        .into());
    }
    if !failed.is_empty() {
        return Err(format!(
            "{} of {total} cases differ: {} (raw output kept under {})",
            failed.len(),
            failed.join(", "),
            scratch.display()
        )
        .into());
    }
    std::fs::remove_dir_all(&scratch)?;
    Ok(())
}
