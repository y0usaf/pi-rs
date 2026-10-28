use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SIDES: [&str; 2] = ["upstream", "pi-rs"];

const COMPARED_EVENTS: [&str; 8] = [
    "agent_start",
    "turn_start",
    "message_end",
    "tool_execution_start",
    "tool_execution_update",
    "tool_execution_end",
    "turn_end",
    "agent_end",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    extension: PathBuf,
    prompt: String,
    turns: Vec<Turn>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Turn {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    tool_calls: Vec<Call>,
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
}

type Script = Arc<Mutex<HashMap<String, (Vec<Turn>, Vec<Value>)>>>;

fn env_path(name: &str) -> Result<PathBuf> {
    match std::env::var_os(name) {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Err(format!("set {name}").into()),
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
    [
        json!({"id": id, "choices": [{"index": 0, "delta": delta, "finish_reason": null}]}),
        json!({"id": id, "choices": [{"index": 0, "delta": {}, "finish_reason": finish}]}),
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
    let next = {
        let mut script = script.lock().map_err(|_| "script lock poisoned")?;
        let (turns, requests) = script
            .get_mut(&label)
            .ok_or_else(|| format!("request for unknown run {label}"))?;
        requests.push(body);
        let index = requests.len() - 1;
        turns.get(index).map(|turn| stream_body(index, turn))
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

fn run(bin: &Path, root: &Path, base_url: &str, extension: &Path, prompt: &str) -> Result<Run> {
    for dir in ["home", "agent", "cwd"] {
        std::fs::create_dir_all(root.join(dir))?;
    }
    let models = json!({"providers": {"fake": {
        "baseUrl": base_url,
        "apiKey": "sk-test",
        "api": "openai-completions",
        "models": [{"id": "fake-test", "contextWindow": 200000, "maxTokens": 8192}],
    }}});
    std::fs::write(root.join("agent/models.json"), models.to_string())?;
    let mut child = Command::new(bin)
        .args([
            "--provider",
            "fake",
            "--model",
            "fake-test",
            "--no-session",
            "--mode",
            "json",
            "-p",
            "-e",
        ])
        .arg(extension)
        .arg(prompt)
        .current_dir(root.join("cwd"))
        .env_clear()
        .env("HOME", root.join("home"))
        .env("PI_CODING_AGENT_DIR", root.join("agent"))
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("PI_OFFLINE", "1")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_TELEMETRY", "0")
        .env("PI_PARITY_LOG", root.join("extension.log"))
        .envs(std::env::var_os("PI_EXTENSION_HOST").map(|host| ("PI_EXTENSION_HOST", host)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let stdout = std::thread::spawn(move || {
        let mut text = String::new();
        stdout.read_to_string(&mut text).map(|_| text)
    });
    let stderr = std::thread::spawn(move || {
        let mut text = String::new();
        stderr.read_to_string(&mut text).map(|_| text)
    });
    let deadline = Instant::now() + Duration::from_secs(120);
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
    let stdout = stdout.join().map_err(|_| "stdout reader panicked")??;
    let stderr = stderr.join().map_err(|_| "stderr reader panicked")??;
    std::fs::write(root.join("stdout.jsonl"), &stdout)?;
    std::fs::write(root.join("stderr.txt"), &stderr)?;
    Ok(Run {
        status,
        stdout,
        stderr,
        log: std::fs::read_to_string(root.join("extension.log")).ok(),
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

fn trace(run: &Run, root: &Path, docs: &Regex) -> Result<Vec<Value>> {
    let root = root.to_string_lossy();
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
            _ => {}
        }
        events.push(canonical(event, &root, docs));
    }
    if let Some(log) = &run.log {
        events.push(canonical(
            json!({"extension_log": log.lines().collect::<Vec<_>>()}),
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

fn problems(side: &str, run: &Run, requests: usize, fixture: &Fixture) -> Vec<String> {
    let mut found = Vec::new();
    if !run.status.success() {
        found.push(format!("{side} exited {}", run.status));
    }
    if !run.stderr.trim().is_empty() {
        found.push(format!("{side} stderr:\n{}", run.stderr.trim_end()));
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
    let bins = [env_path("PI_UPSTREAM_BIN")?, env_path("PI_RS_BIN")?];
    let fixtures_dir = env_path("PI_PARITY_FIXTURES")?;
    let mut fixtures = Vec::new();
    for entry in std::fs::read_dir(&fixtures_dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let fixture: Fixture = serde_json::from_str(&std::fs::read_to_string(&path)?)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let extension = fixtures_dir.join(&fixture.extension);
            if !extension.is_file() {
                return Err(format!(
                    "{}: no extension at {}",
                    path.display(),
                    extension.display()
                )
                .into());
            }
            let name = path
                .file_stem()
                .ok_or("fixture without a name")?
                .to_string_lossy()
                .into_owned();
            fixtures.push((name, extension, fixture));
        }
    }
    if fixtures.is_empty() {
        return Err(format!("no fixtures in {}", fixtures_dir.display()).into());
    }
    fixtures.sort_by(|a, b| a.0.cmp(&b.0));

    let script: Script = Arc::new(Mutex::new(
        fixtures
            .iter()
            .flat_map(|(name, _, fixture)| {
                SIDES.map(|side| {
                    (
                        format!("{name}/{side}"),
                        (fixture.turns.clone(), Vec::new()),
                    )
                })
            })
            .collect(),
    ));
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
    for (name, extension, fixture) in &fixtures {
        let mut found = Vec::new();
        let mut traces = Vec::new();
        for (side, bin) in SIDES.iter().zip(&bins) {
            let label = format!("{name}/{side}");
            let root = scratch.join(&label);
            let base_url = format!("http://127.0.0.1:{port}/{label}/v1");
            match run(bin, &root, &base_url, extension, &fixture.prompt) {
                Ok(run) => {
                    let requests = script
                        .lock()
                        .map_err(|_| "script lock poisoned")?
                        .get(&label)
                        .map_or(0, |(_, requests)| requests.len());
                    found.extend(problems(side, &run, requests, fixture));
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
        failed.push(name.as_str());
    }
    if !failed.is_empty() {
        return Err(format!(
            "{} of {} fixtures differ: {} (raw output kept under {})",
            failed.len(),
            fixtures.len(),
            failed.join(", "),
            scratch.display()
        )
        .into());
    }
    std::fs::remove_dir_all(&scratch)?;
    println!("extensions parity ok: {} fixtures", fixtures.len());
    Ok(())
}
