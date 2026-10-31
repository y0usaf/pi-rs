use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn fail<T>(message: String) -> Result<T> {
    Err(message.into())
}

fn binary() -> Result<PathBuf> {
    let path = match std::env::var("PI_RS_BIN") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => std::env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .map(|dir| Path::new(dir).join("pi-rs"))
            .find(|candidate| {
                std::fs::metadata(candidate)
                    .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            })
            .ok_or("pi-rs not found: set PI_RS_BIN to the built binary")?,
    };
    Ok(std::fs::canonicalize(path)?)
}

/// The stdio server this binary also is, so the check needs no other program: one tool, echo.
fn stdio_server() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = serde_json::from_str(&line)?;
        let id = message.get("id").cloned();
        let reply = match message["method"].as_str() {
            Some("initialize") => Some(json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "stdio-demo", "version": "1.0.0"},
            })),
            Some("tools/list") => Some(json!({
                "tools": [{
                    "name": "echo",
                    "description": "Echo the text back.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"text": {"type": "string"}},
                        "required": ["text"],
                    },
                }],
            })),
            Some("tools/call") => {
                let text = message["params"]["arguments"]["text"]
                    .as_str()
                    .unwrap_or_default();
                Some(json!({
                    "content": [{"type": "text", "text": format!("stdio:{text}")}],
                }))
            }
            _ => None,
        };
        let Some(reply) = reply else { continue };
        let Some(id) = id else { continue };
        writeln!(
            stdout,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "result": reply})
        )?;
        stdout.flush()?;
    }
    Ok(())
}

fn read_request(stream: &mut TcpStream) -> Result<(String, Value)> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let request_line = line.trim_end().to_owned();
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
            length = value.trim().parse::<usize>().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body)?
    };
    Ok((request_line, value))
}

fn respond(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    extra: &str,
) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n{extra}connection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

/// The streamable HTTP server: JSON for everything but a tool call, which answers with an SSE
/// stream, so both shapes the transport reads are exercised. One tool, add.
fn http_server(mut stream: TcpStream) -> Result<()> {
    let (request_line, message) = read_request(&mut stream)?;
    if request_line.starts_with("DELETE") {
        return respond(&mut stream, "200 OK", "application/json", b"", "");
    }
    let id = message.get("id").cloned();
    let reply = match message["method"].as_str() {
        Some("initialize") => Some(json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "http-demo", "version": "2.0.0"},
        })),
        Some("tools/list") => Some(json!({
            "tools": [{
                "name": "add",
                "description": "Add two numbers.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"a": {"type": "number"}, "b": {"type": "number"}},
                    "required": ["a", "b"],
                },
            }],
        })),
        Some("tools/call") => {
            let a = message["params"]["arguments"]["a"].as_i64().unwrap_or(0);
            let b = message["params"]["arguments"]["b"].as_i64().unwrap_or(0);
            let event = json!({
                "jsonrpc": "2.0",
                "id": id.clone(),
                "result": {"content": [{"type": "text", "text": format!("http:{}", a + b)}]},
            });
            let body = format!("event: message\ndata: {event}\n\n");
            return respond(
                &mut stream,
                "200 OK",
                "text/event-stream",
                body.as_bytes(),
                "",
            );
        }
        Some("notifications/initialized") | None => {
            return respond(&mut stream, "202 Accepted", "application/json", b"", "");
        }
        _ => None,
    };
    match (reply, id) {
        (Some(reply), Some(id)) => {
            let body = json!({"jsonrpc": "2.0", "id": id, "result": reply}).to_string();
            respond(
                &mut stream,
                "200 OK",
                "application/json",
                body.as_bytes(),
                "mcp-session-id: session-1\r\n",
            )
        }
        _ => respond(&mut stream, "202 Accepted", "application/json", b"", ""),
    }
}

fn sse(stream: &mut TcpStream, chunk: &Value) -> std::io::Result<()> {
    let payload = format!("data: {chunk}\n\n");
    write!(stream, "{:x}\r\n{payload}\r\n", payload.len())?;
    stream.flush()
}

/// The model: one message asking for both servers' tools, then one reporting what they answered.
fn model_server(mut stream: TcpStream, requests: &Mutex<Vec<Value>>) -> Result<()> {
    let (_, body) = read_request(&mut stream)?;
    if let Ok(mut requests) = requests.lock() {
        requests.push(body.clone());
    }
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let results: Vec<String> = messages
        .iter()
        .filter(|message| message["role"] == "tool")
        .map(|message| {
            let content = &message["content"];
            match content {
                Value::String(text) => text.clone(),
                other => other
                    .as_array()
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|part| part["text"].as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default(),
            }
        })
        .collect();
    stream.write_all(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n",
    )?;
    if results.is_empty() {
        sse(
            &mut stream,
            &json!({"id": "c1", "choices": [{"index": 0, "delta": {"role": "assistant", "tool_calls": [
                {"index": 0, "id": "call_1", "type": "function", "function": {"name": "mcp__stdio__echo", "arguments": "{\"text\":\"hi\"}"}},
                {"index": 1, "id": "call_2", "type": "function", "function": {"name": "mcp__http__add", "arguments": "{\"a\":2,\"b\":3}"}},
            ]}}]}),
        )?;
        sse(
            &mut stream,
            &json!({"id": "c1", "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
        )?;
    } else {
        sse(
            &mut stream,
            &json!({"id": "c2", "choices": [{"index": 0, "delta": {"content": format!("got {}", results.join(", "))}}]}),
        )?;
        sse(
            &mut stream,
            &json!({"id": "c2", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
        )?;
    }
    sse(
        &mut stream,
        &json!({"id": "c", "choices": [], "usage": {"prompt_tokens": 50, "completion_tokens": 8}}),
    )?;
    let done = "data: [DONE]\n\n";
    write!(stream, "{:x}\r\n{done}\r\n", done.len())?;
    stream.write_all(b"0\r\n\r\n")?;
    stream.flush()?;
    Ok(())
}

fn listen<T: FnMut(TcpStream) -> Result<()> + Send + 'static>(handler: T) -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let handler = Arc::new(Mutex::new(handler));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = Arc::clone(&handler);
            std::thread::spawn(move || {
                if let Ok(mut handler) = handler.lock()
                    && let Err(error) = handler(stream)
                    && !format!("{error}").contains("Broken pipe")
                {
                    eprintln!("fake server: {error}");
                }
            });
        }
    });
    Ok(port)
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn run(
    bin: &Path,
    scratch: &Path,
    extra: &[&str],
) -> Result<(String, String, std::process::ExitStatus)> {
    let mut command = Command::new(bin);
    command
        .args([
            "--provider",
            "fake",
            "--model",
            "fake-test",
            "--no-session",
            "-p",
            "echo and add",
        ])
        .args(extra)
        .current_dir(scratch.join("cwd"))
        .env_clear()
        .env("HOME", scratch.join("home"))
        .env("PI_CODING_AGENT_DIR", scratch.join("agent"))
        .env("PATH", "/nonexistent");
    let out = command.output()?;
    Ok((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status,
    ))
}

fn tool_names(request: &Value) -> Vec<String> {
    request["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn system_prompt(request: &Value) -> String {
    match &request["messages"][0]["content"] {
        Value::String(text) => text.clone(),
        parts => parts
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
    }
}

fn main() -> Result<()> {
    if std::env::args().any(|arg| arg == "--stdio-server") {
        return stdio_server();
    }
    let bin = binary()?;
    let scratch = std::env::temp_dir().join(format!("mcp-e2e-{}", std::process::id()));
    for dir in ["home", "agent", "cwd"] {
        std::fs::create_dir_all(scratch.join(dir))?;
    }

    let my_bin = std::env::current_exe()?;
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let model_requests = Arc::clone(&requests);
    let model = listen(move |stream| model_server(stream, &model_requests))?;
    let http = listen(http_server)?;

    write_json(
        &scratch.join("agent/models.json"),
        &json!({"providers": {"fake": {
            "baseUrl": format!("http://127.0.0.1:{model}/v1"),
            "apiKey": "sk-test",
            "api": "openai-completions",
            "models": [{"id": "fake-test", "contextWindow": 200000, "maxTokens": 8192}],
        }}}),
    )?;
    write_json(
        &scratch.join("agent/mcp.json"),
        &json!({"mcpServers": {
            "stdio": {
                "command": my_bin.to_string_lossy(),
                "args": ["--stdio-server"],
                "exposure": "direct",
                "description": "The stdio demo server.",
            },
            "http": {"url": format!("http://127.0.0.1:{http}/mcp"), "exposure": "direct"},
            "indirect": {
                "command": my_bin.to_string_lossy(),
                "args": ["--stdio-server"],
                "description": "A server whose tools stay undeclared.",
            },
        }}),
    )?;

    let (pi_rs_out, stderr, status) = run(&bin, &scratch, &[])?;
    if !status.success() {
        return fail(format!("pi-rs exited {status}: {stderr}"));
    }
    if pi_rs_out.trim() != "got stdio:hi, http:5" {
        return fail(format!(
            "stdout reads {pi_rs_out:?}, not \"got stdio:hi, http:5\""
        ));
    }
    {
        let requests = requests.lock().map_err(|_| "requests lock")?;
        if requests.len() != 2 {
            return fail(format!("the model got {} requests", requests.len()));
        }
        let names = tool_names(&requests[0]);
        let expected = [
            "read",
            "bash",
            "edit",
            "write",
            "mcp__stdio__echo",
            "mcp__http__add",
        ];
        if names != expected {
            return fail(format!(
                "the first request declares {names:?}, not {expected:?}"
            ));
        }
        let prompt = system_prompt(&requests[0]);
        let section = "<mcp_servers>\nMCP servers whose tools are not declared to you.\n- mcp__indirect (codemode): A server whose tools stay undeclared.\n</mcp_servers>";
        if !prompt.contains(section) {
            return fail(format!("the system prompt lacks {section:?}"));
        }
        let tool_results: Vec<&str> = requests[1]["messages"]
            .as_array()
            .map(|messages| {
                messages
                    .iter()
                    .filter(|message| message["role"] == "tool")
                    .filter_map(|message| message["content"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        if tool_results != ["stdio:hi", "http:5"] {
            return fail(format!("the tool results read {tool_results:?}"));
        }
    }
    println!("stdio and http tools ok");

    requests.lock().map_err(|_| "requests lock")?.clear();
    let (stdout, stderr, status) = run(&bin, &scratch, &["--no-mcp"])?;
    if !status.success() {
        return fail(format!("pi-rs --no-mcp exited {status}: {stderr}"));
    }
    {
        let requests = requests.lock().map_err(|_| "requests lock")?;
        let first = requests.first().ok_or("no request under --no-mcp")?;
        let names = tool_names(first);
        if names.iter().any(|name| name.starts_with("mcp__")) {
            return fail(format!("--no-mcp still declares {names:?}"));
        }
        if system_prompt(first).contains("<mcp_servers>") {
            return fail("--no-mcp still writes the mcp_servers section".to_owned());
        }
    }
    if stdout.contains("stdio:hi") || stdout.contains("http:5") {
        return fail(format!("--no-mcp ran an MCP tool: {stdout:?}"));
    }
    println!("--no-mcp ok");

    if let Some(upstream) = std::env::var("PI_TS_BIN")
        .ok()
        .filter(|value| !value.is_empty())
    {
        let (upstream_out, upstream_err, upstream_status) =
            run(Path::new(&upstream), &scratch, &[])?;
        if !upstream_status.success() {
            return fail(format!("pi exited {upstream_status}: {upstream_err}"));
        }
        if upstream_out.trim() != pi_rs_out.trim() {
            return fail(format!(
                "pi answered {:?}, pi-rs answered {:?}",
                upstream_out.trim(),
                pi_rs_out.trim()
            ));
        }
        println!("upstream pi answered the same");
    }

    std::fs::remove_dir_all(&scratch)?;
    println!("mcp e2e ok");
    Ok(())
}
