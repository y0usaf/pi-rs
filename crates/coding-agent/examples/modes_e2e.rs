use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn fail<T>(message: String) -> Result<T> {
    Err(message.into())
}

fn read_request(stream: &mut TcpStream) -> Result<Value> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
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
    let length = length.ok_or("no content-length")?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

fn data(stream: &mut TcpStream, value: &Value) -> std::io::Result<()> {
    let chunk = format!("data: {value}\n\n");
    write!(stream, "{:x}\r\n{chunk}\r\n", chunk.len())?;
    stream.flush()
}

fn start(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.write_all(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n",
    )
}

fn finish_stream(stream: &mut TcpStream) -> std::io::Result<()> {
    data(stream, &json!("[DONE]"))?;
    stream.write_all(b"0\r\n\r\n")
}

fn serve(mut stream: TcpStream, arguments: &str) -> Result<()> {
    let body = read_request(&mut stream)?;
    start(&mut stream)?;
    if body.to_string().contains("\"role\":\"tool\"") {
        for part in ["all", " do", "ne"] {
            std::thread::sleep(Duration::from_millis(120));
            data(
                &mut stream,
                &json!({"id": "c2", "choices": [{"index": 0, "delta": {"content": part}}]}),
            )?;
        }
        data(
            &mut stream,
            &json!({"id": "c2", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
        )?;
        data(
            &mut stream,
            &json!({"id": "c2", "choices": [], "usage": {"prompt_tokens": 1200, "completion_tokens": 12}}),
        )?;
    } else {
        data(
            &mut stream,
            &json!({"id": "c1", "choices": [{"index": 0, "delta": {"role": "assistant", "tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "write", "arguments": ""}}]}}]}),
        )?;
        let cut = arguments
            .char_indices()
            .nth(arguments.chars().count() / 2)
            .map(|(index, _)| index)
            .unwrap_or(arguments.len());
        let (head, tail) = arguments.split_at(cut);
        for part in [head, tail] {
            data(
                &mut stream,
                &json!({"id": "c1", "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": part}}]}}]}),
            )?;
        }
        data(
            &mut stream,
            &json!({"id": "c1", "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
        )?;
        data(
            &mut stream,
            &json!({"id": "c1", "choices": [], "usage": {"prompt_tokens": 600, "completion_tokens": 20}}),
        )?;
    }
    finish_stream(&mut stream)?;
    Ok(())
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn binary() -> Result<PathBuf> {
    let path = match std::env::var("PI_RS_BIN") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => std::env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .map(|dir| Path::new(dir).join("pi-rs"))
            .find(|candidate| executable(candidate))
            .ok_or("pi-rs not found: set PI_RS_BIN to the built binary")?,
    };
    if !executable(&path) {
        return fail(format!(
            "{} is not an executable file (set PI_RS_BIN)",
            path.display()
        ));
    }
    Ok(std::fs::canonicalize(path)?)
}

fn parse_lines(stdout: &[u8]) -> Result<Vec<Value>> {
    let text = String::from_utf8_lossy(stdout).into_owned();
    let mut lines = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        match serde_json::from_str(line) {
            Ok(value) => lines.push(value),
            Err(error) => return fail(format!("stdout line is not JSON ({error}): {line}")),
        }
    }
    Ok(lines)
}

fn need(lines: &[Value], what: &str, found: impl Fn(&Value) -> bool) -> Result<usize> {
    match lines.iter().position(found) {
        Some(index) => Ok(index),
        None => fail(format!("no {what} among {} lines: {lines:#?}", lines.len())),
    }
}

fn stdout_of(bin: &Path, scratch: &Scratch, args: &[&str]) -> Result<(Vec<Value>, String)> {
    let out = Command::new(bin)
        .args(args)
        .current_dir(scratch.root.join("cwd"))
        .env_clear()
        .env("HOME", scratch.root.join("home"))
        .env("PI_CODING_AGENT_DIR", scratch.root.join("agent"))
        .env("PATH", "/nonexistent")
        .output()?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return fail(format!(
            "pi-rs {} exited {}: {stderr}",
            args.join(" "),
            out.status
        ));
    }
    Ok((parse_lines(&out.stdout)?, stderr))
}

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().join(format!("modes-e2e-{}", std::process::id()));
        for dir in ["home", "agent", "cwd"] {
            std::fs::create_dir_all(root.join(dir))?;
        }
        Ok(Self { root })
    }
}

fn json_run(bin: &Path, scratch: &Scratch, written: &Path) -> Result<()> {
    let (lines, _) = stdout_of(
        bin,
        scratch,
        &[
            "--provider",
            "fake",
            "--model",
            "fake-test",
            "--no-session",
            "--mode",
            "json",
            "-p",
            "say hi",
        ],
    )?;
    need(&lines, "agent_start", |line| line["type"] == "agent_start")?;
    need(&lines, "user message_start", |line| {
        line["type"] == "message_start"
            && line["message"]["role"] == "user"
            && line["message"]["content"][0]["text"] == "say hi"
    })?;
    need(&lines, "tool_execution_start for write", |line| {
        line["type"] == "tool_execution_start"
            && line["toolName"] == "write"
            && line["args"]["path"]
                .as_str()
                .is_some_and(|path| path.ends_with("/written.txt"))
    })?;
    let tool_end = need(&lines, "tool_execution_end for call_1", |line| {
        line["type"] == "tool_execution_end"
            && line["toolCallId"] == "call_1"
            && line["isError"] == false
    })?;

    let mut streamed = String::new();
    let mut deltas = 0;
    for line in &lines {
        if line["type"] != "message_update" {
            continue;
        }
        let event = &line["assistantMessageEvent"];
        if event["type"] == "text_delta" && event["contentIndex"] == 0 {
            streamed.push_str(event["delta"].as_str().unwrap_or_default());
            deltas += 1;
        }
    }
    if deltas == 0 {
        return fail(format!(
            "no text_delta among the message_update lines: {lines:#?}"
        ));
    }
    let snapshot = lines
        .iter()
        .find(|line| {
            line["type"] == "message_start"
                && line["message"]["role"] == "assistant"
                && line["message"]["content"][0]["type"] == "text"
        })
        .and_then(|line| line["message"]["content"][0]["text"].as_str())
        .unwrap_or_default();
    // The harness folds a message's last commit into the authoritative `message_end`, so the
    // snapshot and deltas build a prefix of it and `message_end` carries the whole text.
    let streamed = format!("{snapshot}{streamed}");
    if streamed.is_empty() || !"all done".starts_with(&streamed) {
        return fail(format!(
            "the streamed text reads {streamed:?}, which does not start \"all done\""
        ));
    }

    let assistant_end = need(&lines, "assistant message_end", |line| {
        line["type"] == "message_end"
            && line["message"]["role"] == "assistant"
            && line["message"]["content"][0]["text"] == "all done"
            && line["message"]["stopReason"] == "stop"
    })?;
    if assistant_end < tool_end {
        return fail(format!(
            "the assistant message_end at line {assistant_end} precedes the tool_execution_end at line {tool_end}"
        ));
    }
    let agent_end = need(&lines, "agent_end", |line| line["type"] == "agent_end")?;
    if agent_end < assistant_end {
        return fail(format!(
            "agent_end at line {agent_end} precedes the assistant message_end at line {assistant_end}"
        ));
    }
    let written_text = std::fs::read_to_string(written)?;
    if !written_text.contains("hi") {
        return fail(format!("{} holds {written_text:?}", written.display()));
    }
    println!("json mode ok");
    Ok(())
}

fn write_command(stdin: &mut impl Write, command: &str) -> Result<()> {
    writeln!(stdin, "{command}")?;
    stdin.flush()?;
    std::thread::sleep(Duration::from_millis(400));
    Ok(())
}

fn response(lines: &[String], found: impl Fn(&Value) -> bool) -> Option<Value> {
    lines
        .iter()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|line| line["type"] == "response" && found(line))
}

fn by_id(lines: &[String], id: &str) -> Option<Value> {
    response(lines, |line| line["id"] == id)
}

fn rpc_run(bin: &Path, scratch: &Scratch) -> Result<()> {
    let mut child = Command::new(bin)
        .args([
            "--provider",
            "fake",
            "--model",
            "fake-test",
            "--no-session",
            "--mode",
            "rpc",
        ])
        .current_dir(scratch.root.join("cwd"))
        .env_clear()
        .env("HOME", scratch.root.join("home"))
        .env("PI_CODING_AGENT_DIR", scratch.root.join("agent"))
        .env("PATH", "/nonexistent")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let collected = Arc::new(Mutex::new(Vec::new()));
    let reader = {
        let collected = Arc::clone(&collected);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if let Ok(mut lines) = collected.lock() {
                    lines.push(line);
                }
            }
        })
    };
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    write_command(&mut stdin, r#"{"id":"1","type":"get_state"}"#)?;
    write_command(&mut stdin, r#"{"id":"2","type":"get_commands"}"#)?;
    write_command(&mut stdin, r#"{"id":"3","type":"bogus"}"#)?;
    write_command(&mut stdin, "not json")?;
    write_command(&mut stdin, r#"{"id":"4","type":"clear_queue"}"#)?;
    write_command(
        &mut stdin,
        r#"{"id":"5","type":"prompt","message":"say hi"}"#,
    )?;
    std::thread::sleep(Duration::from_secs(3));
    write_command(&mut stdin, r#"{"id":"6","type":"get_last_assistant_text"}"#)?;
    drop(stdin);

    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return fail("pi-rs --mode rpc did not exit within 60s".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    reader.join().map_err(|_| "the stdout reader panicked")?;
    let lines = collected
        .lock()
        .map(|lines| lines.clone())
        .unwrap_or_default();
    if !status.success() {
        return fail(format!("pi-rs --mode rpc exited {status}"));
    }

    let state = by_id(&lines, "1").ok_or("no get_state response")?;
    if state["success"] != true
        || !state["data"]["sessionId"].is_number()
        || state["data"]["thinkingLevel"] != "off"
        || state["data"]["isStreaming"] != false
        || !state["data"]["autoCompactionEnabled"].is_boolean()
        || state["data"]["steeringMode"] != "one-at-a-time"
        || state["data"]["followUpMode"] != "one-at-a-time"
        || state["data"]["sessionFile"].as_str().is_some()
    {
        return fail(format!("get_state answered {}", state));
    }
    let commands = by_id(&lines, "2").ok_or("no get_commands response")?;
    if commands["success"] != true || !commands["data"]["commands"].is_array() {
        return fail(format!("get_commands answered {commands}"));
    }
    let bogus = by_id(&lines, "3").ok_or("no bogus response")?;
    if bogus["success"] != false || bogus["error"] != "Unknown command: bogus" {
        return fail(format!("bogus answered {bogus}"));
    }
    let parsed = response(&lines, |line| line["command"] == "parse").ok_or("no parse response")?;
    if parsed["success"] != false
        || !parsed["error"]
            .as_str()
            .is_some_and(|error| error.starts_with("Failed to parse command:"))
    {
        return fail(format!("the parse response is {parsed}"));
    }
    let clear = by_id(&lines, "4").ok_or("no clear_queue response")?;
    if clear["success"] != true
        || clear["data"]["steering"] != json!([])
        || clear["data"]["followUp"] != json!([])
    {
        return fail(format!("clear_queue answered {clear}"));
    }
    let prompt = by_id(&lines, "5").ok_or("no prompt response")?;
    if prompt["success"] != true || prompt["data"]["disposition"] != "started" {
        return fail(format!("prompt answered {prompt}"));
    }
    let text = by_id(&lines, "6").ok_or("no get_last_assistant_text response")?;
    if text["success"] != true || text["data"]["text"] != "all done" {
        return fail(format!("get_last_assistant_text answered {text}"));
    }
    let events: Vec<Value> = lines
        .iter()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line["type"] != "response")
        .collect();
    if !events.iter().any(|line| {
        line["type"] == "message_update" && line["assistantMessageEvent"]["type"] == "text_delta"
    }) {
        return fail(format!(
            "no text_delta event among the rpc lines: {events:#?}"
        ));
    }
    if !events
        .iter()
        .any(|line| line["type"] == "message_end" && line["message"]["role"] == "assistant")
    {
        return fail(format!(
            "no assistant message_end among the rpc lines: {events:#?}"
        ));
    }
    println!("rpc mode ok");
    Ok(())
}

/// Drives one RPC session and reads its answers by id.
struct Rpc {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    lines: Arc<Mutex<Vec<String>>>,
    reader: Option<std::thread::JoinHandle<()>>,
    counter: usize,
}

impl Rpc {
    fn start(bin: &Path, scratch: &Scratch, args: &[&str]) -> Result<Self> {
        let mut child = Command::new(bin)
            .args(args)
            .current_dir(scratch.root.join("cwd"))
            .env_clear()
            .env("HOME", scratch.root.join("home"))
            .env("PI_CODING_AGENT_DIR", scratch.root.join("agent"))
            .env("PATH", "/nonexistent")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let lines = Arc::new(Mutex::new(Vec::new()));
        let reader = {
            let lines = Arc::clone(&lines);
            std::thread::spawn(move || {
                for line in BufReader::new(stdout)
                    .lines()
                    .map_while(std::result::Result::ok)
                {
                    if let Ok(mut lines) = lines.lock() {
                        lines.push(line);
                    }
                }
            })
        };
        let stdin = child.stdin.take().ok_or("no stdin")?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            lines,
            reader: Some(reader),
            counter: 0,
        })
    }

    /// Send a command and wait for the response carrying its id.
    fn send(&mut self, command: Value) -> Result<Value> {
        self.counter += 1;
        let id = self.counter.to_string();
        let mut command = command;
        command["id"] = json!(id);
        let stdin = self.stdin.as_mut().ok_or("stdin is closed")?;
        writeln!(stdin, "{command}")?;
        stdin.flush()?;
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(found) = self.find(&id) {
                return Ok(found);
            }
            if Instant::now() >= deadline {
                return fail(format!("no response to {command} within 120s"));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn find(&self, id: &str) -> Option<Value> {
        let lines = self.lines.lock().ok()?;
        lines
            .iter()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|line| line["type"] == "response" && line["id"] == id)
    }

    /// The session state, once the run in flight has settled.
    fn idle(&mut self) -> Result<Value> {
        for _ in 0..2400 {
            let state = self.send(json!({"type": "get_state"}))?;
            if state["success"] == true && state["data"]["isStreaming"] == false {
                return Ok(state["data"].clone());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        fail("the session never went idle".to_owned())
    }

    fn ok(&mut self, command: Value) -> Result<Value> {
        let answer = self.send(command.clone())?;
        if answer["success"] != true {
            return fail(format!("{command} answered {answer}"));
        }
        Ok(answer["data"].clone())
    }

    fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(120);
        while self.child.try_wait()?.is_none() {
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                return fail("pi-rs --mode rpc did not exit within 120s".to_owned());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        let status = self.child.wait()?;
        if !status.success() {
            return fail(format!("pi-rs --mode rpc exited {status}"));
        }
        Ok(())
    }
}

fn same_text(entry: &Value, text: &str) -> bool {
    entry["model"][0]["content"][0]["text"] == text
}

fn rpc_session_run(bin: &Path, scratch: &Scratch) -> Result<()> {
    let mut rpc = Rpc::start(
        bin,
        scratch,
        &[
            "--provider",
            "fake",
            "--model",
            "fake-test",
            "--mode",
            "rpc",
        ],
    )?;

    // Queue modes, auto-compaction and auto-retry are saved and reported back.
    for command in [
        json!({"type": "set_steering_mode", "mode": "all"}),
        json!({"type": "set_follow_up_mode", "mode": "one-at-a-time"}),
        json!({"type": "set_auto_compaction", "enabled": false}),
        json!({"type": "set_auto_retry", "enabled": false}),
        json!({"type": "abort_retry"}),
    ] {
        rpc.ok(command)?;
    }
    let state = rpc.idle()?;
    if state["steeringMode"] != "all"
        || state["followUpMode"] != "one-at-a-time"
        || state["autoCompactionEnabled"] != false
    {
        return fail(format!("get_state reports {state}"));
    }
    let source = state["sessionFile"]
        .as_str()
        .ok_or("get_state named no sessionFile")?
        .to_owned();

    // An empty queue empties to two empty lists.
    let queue = rpc.ok(json!({"type": "clear_queue"}))?;
    if queue["steering"] != json!([]) || queue["followUp"] != json!([]) {
        return fail(format!("clear_queue answered {queue}"));
    }

    // A run, then the reads over it.
    rpc.ok(json!({"type": "prompt", "message": "say hi"}))?;
    let state = rpc.idle()?;
    let count = state["messageCount"].as_u64().ok_or("no messageCount")?;
    if count == 0 || state["pendingMessageCount"] != 0 {
        return fail(format!("get_state after a run reports {state}"));
    }

    let tree = rpc.ok(json!({"type": "get_tree"}))?;
    let nodes = tree["tree"].as_array().ok_or("get_tree returned no tree")?;
    let mut user = 0;
    let mut last = None;
    fn walk(nodes: &[Value], user: &mut usize, last: &mut Option<Value>) {
        for node in nodes {
            let entry = &node["entry"];
            if same_text(entry, "say hi") {
                *user += 1;
            }
            if same_text(entry, "all done") {
                *last = Some(entry["id"].clone());
            }
            if let Some(children) = node["children"].as_array() {
                walk(children, user, last);
            }
        }
    }
    walk(nodes, &mut user, &mut last);
    let last = last.ok_or("get_tree holds no assistant entry")?;
    if user != 1 || tree["leafId"] != last {
        return fail(format!(
            "get_tree reports leaf {:?} for {user} user entries",
            tree["leafId"]
        ));
    }

    let forks = rpc.ok(json!({"type": "get_fork_messages"}))?;
    let messages = forks["messages"].as_array().ok_or("no fork messages")?;
    if messages.len() != 1 || messages[0]["text"] != "say hi" {
        return fail(format!("get_fork_messages answered {forks}"));
    }

    let stats = rpc.ok(json!({"type": "get_session_stats"}))?;
    if stats["userMessages"] != 1
        || stats["assistantMessages"] != 2
        || stats["toolCalls"] != 1
        || stats["toolResults"] != 1
        || stats["totalMessages"] != 5
        || stats["sessionFile"] != json!(source)
        || stats["tokens"]["input"].as_u64().unwrap_or(0) == 0
        || stats["tokens"]["output"].as_u64().unwrap_or(0) == 0
        || stats["contextUsage"]["contextWindow"] != 200000
    {
        return fail(format!("get_session_stats answered {stats}"));
    }

    // A steering message queued behind a run is handed back by clear_queue.
    rpc.ok(json!({"type": "prompt", "message": "second"}))?;
    rpc.ok(json!({"type": "steer", "message": "queued note"}))?;
    let queue = rpc.ok(json!({"type": "clear_queue"}))?;
    if queue["steering"] != json!(["queued note"]) {
        return fail(format!("clear_queue during a run answered {queue}"));
    }
    let settled = rpc.idle()?;
    if settled["pendingMessageCount"] != 0 {
        return fail(format!("a cleared queue still holds messages: {settled}"));
    }

    // A model that does not reason has one thinking level and nothing to cycle to.
    let levels = rpc.ok(json!({"type": "get_available_thinking_levels"}))?;
    if levels["levels"] != json!(["off"]) {
        return fail(format!("get_available_thinking_levels answered {levels}"));
    }
    if rpc.ok(json!({"type": "cycle_thinking_level"}))? != Value::Null {
        return fail(
            "cycle_thinking_level answered a level for a model that does not reason".to_owned(),
        );
    }

    // cycle_model walks the catalog; the second model reasons, so the levels open up.
    let cycled = rpc.ok(json!({"type": "cycle_model"}))?;
    if cycled["model"]["id"] != "fake-thinky" || cycled["isScoped"] != false {
        return fail(format!("cycle_model answered {cycled}"));
    }
    let levels = rpc.ok(json!({"type": "get_available_thinking_levels"}))?;
    if levels["levels"] != json!(["off", "minimal", "low", "medium", "high"]) {
        return fail(format!("get_available_thinking_levels answered {levels}"));
    }
    let cycled = rpc.ok(json!({"type": "cycle_thinking_level"}))?;
    if cycled["level"] != "minimal" {
        return fail(format!("cycle_thinking_level answered {cycled}"));
    }

    // clone writes the active path to a new file and switches to it.
    let before = rpc.idle()?;
    let cloned = rpc.ok(json!({"type": "clone"}))?;
    if cloned["cancelled"] != false {
        return fail(format!("clone answered {cloned}"));
    }
    let after = rpc.idle()?;
    if after["sessionFile"] == json!(source) || after["messageCount"] != before["messageCount"] {
        return fail(format!("clone left the session at {after}"));
    }
    let clone_path = after["sessionFile"].clone();

    // fork stops before the chosen user message and hands its text back.
    let forks = rpc.ok(json!({"type": "get_fork_messages"}))?;
    let first = forks["messages"][0]["entryId"].clone();
    let forked = rpc.ok(json!({"type": "fork", "entryId": first}))?;
    if forked["text"] != "say hi" || forked["cancelled"] != false {
        return fail(format!("fork answered {forked}"));
    }
    let after = rpc.idle()?;
    if after["sessionFile"] == clone_path || after["sessionFile"] == json!(source) {
        return fail(format!("fork left the session at {after}"));
    }
    if after["messageCount"] != 0 {
        return fail(format!("forking before the first message left {after}"));
    }
    let forks = rpc.ok(json!({"type": "get_fork_messages"}))?;
    if forks["messages"] != json!([]) {
        return fail(format!(
            "a session forked before its first entry holds {forks}"
        ));
    }

    // new_session opens an empty file of its own.
    let fresh = rpc.ok(json!({"type": "new_session"}))?;
    if fresh["cancelled"] != false {
        return fail(format!("new_session answered {fresh}"));
    }
    let after = rpc.idle()?;
    if after["messageCount"] != 0 || after["sessionFile"] == json!(source) {
        return fail(format!("new_session left the session at {after}"));
    }

    // switch_session goes back to the first file, with everything it held.
    let switched = rpc.ok(json!({"type": "switch_session", "sessionPath": source}))?;
    if switched["cancelled"] != false {
        return fail(format!("switch_session answered {switched}"));
    }
    let after = rpc.idle()?;
    if after["sessionFile"] != json!(source) || after["messageCount"] != before["messageCount"] {
        return fail(format!("switch_session left the session at {after}"));
    }

    // A path that is not there, and an entry that is not in the session, are refused.
    let missing =
        rpc.send(json!({"type": "switch_session", "sessionPath": "/nonexistent/nope.db"}))?;
    if missing["success"] != false
        || !missing["error"]
            .as_str()
            .is_some_and(|e| e.starts_with("Session not found:"))
    {
        return fail(format!(
            "switch_session to a missing file answered {missing}"
        ));
    }
    let bogus = rpc.send(json!({"type": "fork", "entryId": 999999}))?;
    if bogus["success"] != false || bogus["error"] != "Invalid entry ID for forking" {
        return fail(format!("fork of a missing entry answered {bogus}"));
    }

    rpc.finish()?;
    println!("rpc session mode ok");
    Ok(())
}

fn main() -> Result<()> {
    let bin = binary()?;
    let scratch = Scratch::new()?;
    let written = scratch.root.join("cwd").join("written.txt");
    let arguments = json!({"path": written.to_string_lossy(), "content": "hi"}).to_string();

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let arguments = Arc::new(arguments);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let arguments = Arc::clone(&arguments);
            std::thread::spawn(move || {
                if let Err(error) = serve(stream, &arguments) {
                    eprintln!("fake server: {error}");
                }
            });
        }
    });

    let models = json!({"providers": {"fake": {
        "baseUrl": format!("http://127.0.0.1:{port}/v1"),
        "apiKey": "sk-test",
        "api": "openai-completions",
        "models": [{"id": "fake-test", "contextWindow": 200000, "maxTokens": 8192}, {"id": "fake-thinky", "contextWindow": 200000, "maxTokens": 8192, "reasoning": true}],
    }}});
    std::fs::write(
        scratch.root.join("agent/models.json"),
        serde_json::to_vec(&models)?,
    )?;

    json_run(&bin, &scratch, &written)?;
    rpc_run(&bin, &scratch)?;
    rpc_session_run(&bin, &scratch)?;
    std::fs::remove_dir_all(&scratch.root)?;
    println!("modes e2e ok");
    Ok(())
}
