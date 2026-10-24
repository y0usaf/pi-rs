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
        .current_dir(&scratch.cwd)
        .env_clear()
        .env("HOME", &scratch.home)
        .env("PI_CODING_AGENT_DIR", &scratch.agent)
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
    home: PathBuf,
    agent: PathBuf,
    cwd: PathBuf,
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
    let streamed = format!("{snapshot}{streamed}");
    if streamed != "all done" {
        return fail(format!(
            "the streamed text reads {streamed:?}, not \"all done\""
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
        .current_dir(&scratch.cwd)
        .env_clear()
        .env("HOME", &scratch.home)
        .env("PI_CODING_AGENT_DIR", &scratch.agent)
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
    if clear["success"] != false
        || clear["error"] != "pi-rs: clear_queue is not implemented in RPC mode"
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

fn main() -> Result<()> {
    let bin = binary()?;
    let scratch = Scratch {
        home: std::env::temp_dir()
            .join(format!("modes-e2e-{}", std::process::id()))
            .join("home"),
        agent: std::env::temp_dir()
            .join(format!("modes-e2e-{}", std::process::id()))
            .join("agent"),
        cwd: std::env::temp_dir()
            .join(format!("modes-e2e-{}", std::process::id()))
            .join("cwd"),
    };
    for dir in [&scratch.home, &scratch.agent, &scratch.cwd] {
        std::fs::create_dir_all(dir)?;
    }
    let written = scratch.cwd.join("written.txt");
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
        "models": [{"id": "fake-test", "contextWindow": 200000, "maxTokens": 8192}],
    }}});
    std::fs::write(
        scratch.agent.join("models.json"),
        serde_json::to_vec(&models)?,
    )?;

    json_run(&bin, &scratch, &written)?;
    rpc_run(&bin, &scratch)?;
    println!("modes e2e ok");
    Ok(())
}
