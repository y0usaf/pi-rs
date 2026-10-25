//! Drives the codemode sandbox end to end through the built `pi-rs` binary: the hidden
//! `--codemode-vm` child's wire format, the tools bridge, the deadline, and the abort.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pi_codemode::host::{CallStatus, ErrorKind, ExecuteOptions};
use pi_codemode::{DeclaredTool, OutputItem, execute, parse_source};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

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

/// The start message the host writes, for the wire-format check.
fn start(code: &str) -> Value {
    json!({
        "code": code,
        "tools": [],
        "globals": [],
        "memoryLimitBytes": 268435456,
        "store": {},
    })
}

/// Messages the child reports for one script, read straight off the pipe.
fn pipe(bin: &Path, code: &str) -> Result<Vec<Value>> {
    let mut child = Command::new(bin)
        .arg("--codemode-vm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    writeln!(stdin, "{}", start(code))?;
    drop(stdin);
    let out = child.wait_with_output()?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return fail(format!(
            "pi-rs --codemode-vm exited {}: {stderr}",
            out.status
        ));
    }
    let mut lines = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        match serde_json::from_str(line) {
            Ok(value) => lines.push(value),
            Err(error) => return fail(format!("a stdout line is not JSON ({error}): {line}")),
        }
    }
    Ok(lines)
}

struct Run {
    result: pi_codemode::ScriptResult,
    elapsed: Duration,
}

fn run_script(
    bin: &Path,
    code: &str,
    tools: &[DeclaredTool],
    tool: pi_codemode::host::ToolExecute<'_>,
    timeout_ms: Option<u64>,
    memory_limit_bytes: Option<usize>,
    cancelled: Option<&dyn Fn() -> bool>,
) -> Result<Run> {
    let started = Instant::now();
    let result = execute(
        bin,
        code,
        tools,
        &[],
        &BTreeMap::new(),
        ExecuteOptions {
            timeout_ms,
            memory_limit_bytes,
            cancelled,
            execute: &mut *tool,
        },
    );
    Ok(Run {
        result,
        elapsed: started.elapsed(),
    })
}

fn text_of(output: &[OutputItem]) -> Vec<String> {
    output
        .iter()
        .filter_map(|item| match item {
            OutputItem::Text { text, .. } => Some(text.clone()),
            OutputItem::Image { .. } => None,
        })
        .collect()
}

/// `--codemode-vm` processes still running, from /proc, so a leaked child fails the check.
fn vm_processes() -> Vec<String> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return found;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ");
        if cmdline.contains("--codemode-vm") {
            found.push(format!("{}: {cmdline}", name.to_string_lossy()));
        }
    }
    found
}

fn no_leftovers(what: &str) -> Result<()> {
    let running = vm_processes();
    if !running.is_empty() {
        return fail(format!("{what} left a codemode child behind: {running:?}"));
    }
    Ok(())
}

fn wire_format(bin: &Path) -> Result<()> {
    let lines = pipe(
        bin,
        "text(\"hello\");\nconsole.log(\"logged\", { a: 1 });\nreturn { total: 42 };",
    )?;
    let expected = json!([
        {"type": "output", "item": {"type": "text", "text": "hello"}},
        {"type": "output", "item": {"type": "text", "text": "logged {\"a\":1}", "console": true}},
        {"type": "done", "ok": true, "value": "{\"total\":42}", "writes": "[]"},
    ]);
    if lines != expected.as_array().cloned().unwrap_or_default() {
        return fail(format!(
            "the child reported {lines:#?}, expected {expected:#?}"
        ));
    }
    no_leftovers("the wire-format run")
}

fn tools_bridge(bin: &Path) -> Result<()> {
    let tools = [DeclaredTool::new("read", "Read a file")];
    let mut tool = |name: &str, args: Option<&Value>| match name {
        "read" => {
            let path = args
                .and_then(|args| args.get("path"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            Ok(Some(Value::String(format!("contents of {path}"))))
        }
        other => Err(format!("Unknown tool \"{other}\"")),
    };
    let run = run_script(
        bin,
        "const a = await tools.read({ path: \"a.txt\" });\nconst b = await tools[\"read\"]({ path: \"b.txt\" });\ntext(a);\ntext(b);\nconsole.log(ALL_TOOLS.map((t) => t.name).join(\",\"), ALL_TOOLS.length);",
        &tools,
        &mut tool,
        Some(10_000),
        None,
        None,
    )?;
    if !run.result.ok {
        return fail(format!("the tools script failed: {:#?}", run.result.error));
    }
    let expected = vec![
        "contents of a.txt".to_owned(),
        "contents of b.txt".to_owned(),
        "read 1".to_owned(),
    ];
    if text_of(&run.result.output) != expected {
        return fail(format!(
            "the output reads {:#?}, expected {expected:?}",
            text_of(&run.result.output)
        ));
    }
    let calls: Vec<(&str, &str)> = run
        .result
        .calls
        .iter()
        .map(|call| (call.name.as_str(), call.status.name()))
        .collect();
    if calls != [("read", "ok"), ("read", "ok")] {
        return fail(format!("the call records read {calls:?}"));
    }
    no_leftovers("the tools run")
}

fn failed_tool(bin: &Path) -> Result<()> {
    let tools = [DeclaredTool::new("read", "Read a file")];
    let mut tool = |_: &str, _: Option<&Value>| Err("File not found: nope.txt".to_owned());
    let run = run_script(
        bin,
        "try {\n  await tools.read({ path: \"nope.txt\" });\n} catch (error) {\n  text(error.message);\n}\nawait tools.read({ path: \"again.txt\" });",
        &tools,
        &mut tool,
        Some(10_000),
        None,
        None,
    )?;
    if run.result.ok {
        return fail("a script whose call failed did not fail".to_owned());
    }
    let expected = vec!["File not found: nope.txt".to_owned()];
    if text_of(&run.result.output) != expected {
        return fail(format!(
            "the output reads {:#?}, expected {expected:?}",
            text_of(&run.result.output)
        ));
    }
    let error = run.result.error.as_ref().ok_or("no error")?;
    if error.kind != ErrorKind::Script || error.message != "File not found: nope.txt" {
        return fail(format!("the script error reads {error:#?}"));
    }
    if run
        .result
        .calls
        .iter()
        .any(|call| call.status != CallStatus::Error)
    {
        return fail(format!("the call records read {:#?}", run.result.calls));
    }
    no_leftovers("the failed-tool run")
}

fn unknown_tool(bin: &Path) -> Result<()> {
    let mut tool = |_: &str, _: Option<&Value>| Ok(None);
    let run = run_script(
        bin,
        "await tools.reaad({ path: \"a.txt\" });",
        &[DeclaredTool::new("read", "Read a file")],
        &mut tool,
        Some(10_000),
        None,
        None,
    )?;
    let error = run.result.error.as_ref().ok_or("no error")?;
    let expected = "tools.reaad does not exist. Available: read. ALL_TOOLS lists every tool; searchTools(query) finds tools by topic. Check for a member with \"reaad\" in tools.";
    if error.message != expected {
        return fail(format!("the typo error reads {error:?}"));
    }
    no_leftovers("the typo run")
}

fn stalled(bin: &Path) -> Result<()> {
    let mut tool = |_: &str, _: Option<&Value>| Ok(None);
    let run = run_script(
        bin,
        "await new Promise(() => {});",
        &[],
        &mut tool,
        Some(30_000),
        None,
        None,
    )?;
    let error = run.result.error.as_ref().ok_or("no error")?;
    let expected = "The script is waiting on a promise that can never settle: no tool call is pending, and timers do not exist here.";
    if error.message != expected {
        return fail(format!("the stalled error reads {error:?}"));
    }
    no_leftovers("the stalled run")
}

fn deadline(bin: &Path) -> Result<()> {
    let mut tool = |_: &str, _: Option<&Value>| Ok(None);
    let run = run_script(
        bin,
        "while (true) {}",
        &[],
        &mut tool,
        Some(300),
        None,
        None,
    )?;
    let error = run.result.error.as_ref().ok_or("no error")?;
    if error.kind != ErrorKind::Timeout || error.message != "Execution timed out after 300 ms" {
        return fail(format!("the deadline error reads {error:#?}"));
    }
    if run.elapsed < Duration::from_millis(280) || run.elapsed > Duration::from_secs(10) {
        return fail(format!("the deadline took {:?}", run.elapsed));
    }
    no_leftovers("the deadline run")
}

fn abort(bin: &Path) -> Result<()> {
    let started = Instant::now();
    let cancelled = move || started.elapsed() > Duration::from_millis(200);
    let mut tool = |_: &str, _: Option<&Value>| Ok(None);
    let run = run_script(
        bin,
        "while (true) {}",
        &[],
        &mut tool,
        Some(30_000),
        None,
        Some(&cancelled),
    )?;
    let error = run.result.error.as_ref().ok_or("no error")?;
    if error.kind != ErrorKind::Aborted || error.message != "Execution aborted" {
        return fail(format!("the abort error reads {error:#?}"));
    }
    if run.elapsed > Duration::from_secs(5) {
        return fail(format!("the abort took {:?}", run.elapsed));
    }
    no_leftovers("the abort run")
}

fn memory_limit(bin: &Path) -> Result<()> {
    let mut tool = |_: &str, _: Option<&Value>| Ok(None);
    let run = run_script(
        bin,
        "const chunks = [];\nfor (let i = 0; i < 100000; i++) chunks.push(new Array(1000).fill(i));",
        &[],
        &mut tool,
        Some(30_000),
        Some(32 * 1024 * 1024),
        None,
    )?;
    let error = run.result.error.as_ref().ok_or("no error")?;
    if run.result.ok || !error.message.to_lowercase().contains("out of memory") {
        return fail(format!("the memory limit error reads {error:#?}"));
    }
    no_leftovers("the memory run")
}

fn store_writes(bin: &Path) -> Result<()> {
    let mut tool = |_: &str, _: Option<&Value>| Ok(None);
    let run = run_script(
        bin,
        "store(\"kept\", { n: 1 });\nstore(\"dropped\", undefined);\nstore(\"big\", \"x\".repeat(300000));",
        &[],
        &mut tool,
        Some(10_000),
        None,
        None,
    )?;
    if run.result.ok {
        return fail("a store() past the value limit did not fail".to_owned());
    }
    let error = run.result.error.as_ref().ok_or("no error")?;
    if !error.message.starts_with("store(\"big\") value has ") {
        return fail(format!("the store limit error reads {error:?}"));
    }
    let mut tool = |_: &str, _: Option<&Value>| Ok(None);
    let run = run_script(
        bin,
        "store(\"kept\", { n: 1 });\nstore(\"dropped\", undefined);",
        &[],
        &mut tool,
        Some(10_000),
        None,
        None,
    )?;
    if !run.result.ok {
        return fail(format!("the store script failed: {:#?}", run.result.error));
    }
    let writes = &run.result.store_writes;
    if writes.set.get("kept") != Some(&json!({"n": 1})) || writes.delete != ["dropped"] {
        return fail(format!("the store writes read {writes:#?}"));
    }
    no_leftovers("the store run")
}

fn source_options(bin: &Path) -> Result<()> {
    let parsed = parse_source(
        "// @options: {\"max_output_tokens\": 2000, \"timeout_ms\": 30000}\ntext(1);",
    )?;
    if parsed.options.max_output_tokens != Some(2000) || parsed.options.timeout_ms != Some(30000) {
        return fail(format!("the options parse to {:#?}", parsed.options));
    }
    if !parsed.code.starts_with("\n") || !parsed.code.contains("text(1);") {
        return fail(format!("the code reads {:?}", parsed.code));
    }
    let misplaced = parse_source("text(1);\n// @options: {}");
    match misplaced {
        Err(message) => {
            let expected =
                "The // @options: line must be the first line of the input; found one on line 2";
            if message != expected {
                return fail(format!("the misplaced options error reads {message:?}"));
            }
        }
        Ok(_) => return fail("a misplaced @options line was accepted".to_owned()),
    }
    if parse_source("  ").is_ok() {
        return fail("empty source was accepted".to_owned());
    }
    let mut tool = |_: &str, _: Option<&Value>| Ok(None);
    let run = run_script(bin, &parsed.code, &[], &mut tool, Some(10_000), None, None)?;
    if !run.result.ok || text_of(&run.result.output) != ["1"] {
        return fail(format!("the @options script failed: {:#?}", run.result));
    }
    no_leftovers("the @options run")
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

/// The script the model asks for: it reads a file through the agent's own read tool.
fn script() -> String {
    "const content = await tools.read({ path: \"hello.txt\" });\ntext(content);\nreturn { length: content.length };"
        .to_owned()
}

fn serve(mut stream: TcpStream, requests: Arc<Mutex<Vec<Value>>>) -> Result<()> {
    let body = read_request(&mut stream)?;
    let has_tool_result = body["messages"]
        .as_array()
        .is_some_and(|messages| messages.iter().any(|message| message["role"] == "tool"));
    if let Ok(mut requests) = requests.lock() {
        requests.push(body.clone());
    }
    stream.write_all(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n",
    )?;
    if has_tool_result {
        data(
            &mut stream,
            &json!({"id": "c2", "choices": [{"index": 0, "delta": {"content": "done"}}]}),
        )?;
        data(
            &mut stream,
            &json!({"id": "c2", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
        )?;
        data(
            &mut stream,
            &json!({"id": "c2", "choices": [], "usage": {"prompt_tokens": 900, "completion_tokens": 3}}),
        )?;
    } else {
        let arguments = json!({"code": script()}).to_string();
        data(
            &mut stream,
            &json!({"id": "c1", "choices": [{"index": 0, "delta": {"role": "assistant", "tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "codemode", "arguments": ""}}]}}]}),
        )?;
        data(
            &mut stream,
            &json!({"id": "c1", "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": arguments}}]}}]}),
        )?;
        data(
            &mut stream,
            &json!({"id": "c1", "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
        )?;
        data(
            &mut stream,
            &json!({"id": "c1", "choices": [], "usage": {"prompt_tokens": 800, "completion_tokens": 20}}),
        )?;
    }
    data(&mut stream, &json!("[DONE]"))?;
    stream.write_all(b"0\r\n\r\n")?;
    Ok(())
}

/// `pi-rs -p` with a fake model whose only call is `codemode` running a script that calls `read`.
fn agent_run(bin: &Path) -> Result<()> {
    let root = std::env::temp_dir().join(format!("codemode-e2e-{}", std::process::id()));
    for dir in ["home", "agent", "cwd"] {
        std::fs::create_dir_all(root.join(dir))?;
    }
    std::fs::write(root.join("cwd/hello.txt"), "hello from the file\n")?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let requests = Arc::clone(&served);
            std::thread::spawn(move || {
                if let Err(error) = serve(stream, requests) {
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
    std::fs::write(root.join("agent/models.json"), serde_json::to_vec(&models)?)?;
    let out = Command::new(bin)
        .args([
            "--provider",
            "fake",
            "--model",
            "fake-test",
            "--no-session",
            "--tools",
            "+codemode",
            "-p",
            "go",
        ])
        .current_dir(root.join("cwd"))
        .env_clear()
        .env("HOME", root.join("home"))
        .env("PI_CODING_AGENT_DIR", root.join("agent"))
        .env("PATH", "/nonexistent")
        .output()?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return fail(format!("pi-rs -p exited {}: {stderr}", out.status));
    }
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if stdout.trim() != "done" {
        return fail(format!("pi-rs -p printed {stdout:?}"));
    }
    let requests = requests
        .lock()
        .map(|requests| requests.clone())
        .unwrap_or_default();
    if requests.len() < 2 {
        return fail(format!(
            "the model saw {} request(s), expected the tool result to be sent back: {stderr}",
            requests.len()
        ));
    }
    let tools = requests[0]["tools"].as_array().cloned().unwrap_or_default();
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect();
    if !names.contains(&"codemode") {
        return fail(format!(
            "the first request declares {names:?}, without codemode"
        ));
    }
    let codemode = tools
        .iter()
        .find(|tool| tool["function"]["name"] == "codemode")
        .ok_or("no codemode tool")?;
    let description = codemode["function"]["description"]
        .as_str()
        .unwrap_or_default();
    if !description.starts_with("Run JavaScript that calls other tools.") {
        return fail(format!("the codemode description reads {description:?}"));
    }
    if codemode["function"]["parameters"]["required"] != json!(["code"]) {
        return fail(format!(
            "the codemode schema reads {}",
            codemode["function"]["parameters"]
        ));
    }
    let tool_results: Vec<&Value> = requests[1]["messages"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .filter(|message| message["role"] == "tool")
                .collect()
        })
        .unwrap_or_default();
    let result = tool_results
        .first()
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default();
    if !result.starts_with("Script completed\nWall time ") {
        return fail(format!("the tool result reads {result:?}"));
    }
    for expected in [
        "hello from the file",
        "==> text 1/2 <==",
        "==> text 2/2 <==",
        "{\"length\":20}",
    ] {
        if !result.contains(expected) {
            return fail(format!("the tool result lacks {expected:?}: {result:?}"));
        }
    }
    std::fs::remove_dir_all(&root)?;
    no_leftovers("the agent run")?;
    Ok(())
}

/// The two `codemode.mode` values: `only` declares codemode alone and lists every tool in its
/// description; `on` keeps the direct tools declared and says how a script calls them.
fn loadout_modes(bin: &Path) -> Result<()> {
    for (mode, expected) in [("only", 1usize), ("on", 5usize)] {
        let root =
            std::env::temp_dir().join(format!("codemode-modes-{mode}-{}", std::process::id()));
        for dir in ["home", "agent", "cwd"] {
            std::fs::create_dir_all(root.join(dir))?;
        }
        std::fs::write(root.join("cwd/hello.txt"), "hello\n")?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let served = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let requests = Arc::clone(&served);
                std::thread::spawn(move || {
                    if let Err(error) = serve(stream, requests) {
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
        std::fs::write(root.join("agent/models.json"), serde_json::to_vec(&models)?)?;
        let settings = json!({"defaultTools": ["+codemode"], "codemode": {"mode": mode}});
        std::fs::write(
            root.join("agent/settings.json"),
            serde_json::to_vec(&settings)?,
        )?;
        let out = Command::new(bin)
            .args([
                "--provider",
                "fake",
                "--model",
                "fake-test",
                "--no-session",
                "-p",
                "go",
            ])
            .current_dir(root.join("cwd"))
            .env_clear()
            .env("HOME", root.join("home"))
            .env("PI_CODING_AGENT_DIR", root.join("agent"))
            .env("PATH", "/nonexistent")
            .output()?;
        if !out.status.success() {
            return fail(format!(
                "pi-rs -p (mode {mode}) exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let requests = requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default();
        let first = requests.first().ok_or("the model saw no request")?;
        let tools = first["tools"].as_array().cloned().unwrap_or_default();
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool["function"]["name"].as_str())
            .collect();
        if names.len() != expected {
            return fail(format!(
                "mode {mode} declares {names:?}, expected {expected} tools"
            ));
        }
        let description = |name: &str| -> String {
            tools
                .iter()
                .find(|tool| tool["function"]["name"] == name)
                .and_then(|tool| tool["function"]["description"].as_str())
                .unwrap_or_default()
                .to_owned()
        };
        if mode == "only" {
            if names != ["codemode"] {
                return fail(format!("mode only declares {names:?}"));
            }
            let codemode = description("codemode");
            for expected in [
                "Nested tools:",
                "### `read`",
                "### `bash`",
                "declare const tools: { read(args: {",
            ] {
                if !codemode.contains(expected) {
                    return fail(format!("mode only's description lacks {expected:?}"));
                }
            }
            let system = first["messages"][0]["content"].as_str().unwrap_or_default();
            let tools_section = system
                .split("<tools>")
                .nth(1)
                .and_then(|rest| rest.split("</tools>").next())
                .unwrap_or_default();
            if !tools_section.contains("- codemode: Run JavaScript that calls other tools")
                || tools_section.contains("- read:")
            {
                return fail(format!("mode only's system prompt lists {tools_section:?}"));
            }
            // The script still reaches read although the request never declares it.
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            if stdout.trim() != "done" {
                return fail(format!("mode only's run printed {stdout:?}"));
            }
            let tool_result = requests
                .get(1)
                .and_then(|request| request["messages"].as_array().cloned())
                .and_then(|messages| {
                    messages
                        .into_iter()
                        .find(|message| message["role"] == "tool")
                })
                .and_then(|message| message["content"].as_str().map(str::to_owned))
                .unwrap_or_default();
            if !tool_result.contains("hello") || !tool_result.starts_with("Script completed") {
                return fail(format!(
                    "the hidden read did not reach the script: {tool_result:?}"
                ));
            }
        } else {
            if !names.contains(&"read") || !names.contains(&"codemode") {
                return fail(format!("mode on declares {names:?}"));
            }
            let codemode = description("codemode");
            if codemode.contains("Nested tools:") {
                return fail(format!(
                    "mode on lists direct tools in the description: {codemode}"
                ));
            }
            let read = description("read");
            if !read.contains("Codemode: `tools.read(args)` resolves to a string.") {
                return fail(format!("mode on's read description reads {read:?}"));
            }
        }
        std::fs::remove_dir_all(&root)?;
    }
    no_leftovers("the loadout runs")
}

fn main() -> Result<()> {
    let bin = binary()?;
    wire_format(&bin)?;
    tools_bridge(&bin)?;
    failed_tool(&bin)?;
    unknown_tool(&bin)?;
    stalled(&bin)?;
    deadline(&bin)?;
    abort(&bin)?;
    memory_limit(&bin)?;
    store_writes(&bin)?;
    source_options(&bin)?;
    agent_run(&bin)?;
    loadout_modes(&bin)?;
    println!("codemode e2e ok");
    Ok(())
}
