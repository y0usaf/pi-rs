use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const PACKAGE: &str = r#"import { Type } from "typebox";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI): void {
	pi.on("session_start", () => {});
	pi.on("message_end", () => undefined);
	pi.registerCommand("ext-check", {
		description: JSON.stringify(Type.Object({ text: Type.String({ description: "what to echo" }) })),
		handler: async () => "ok",
	});
}
"#;

const CLI: &str = r#"import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI): void {
	pi.on("agent_end", () => {});
	pi.registerCommand("cli-check", { description: "from --extension", handler: async () => "ok" });
}
"#;

const BROKEN: &str = r#"export default function (): void {
	throw new Error("this extension throws");
}
"#;

const UNSUPPORTED: &str = r#"import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI): void {
	pi.registerShortcut("ctrl+k", { handler: () => {} });
}
"#;

fn fail<T>(message: String) -> Result<T> {
    Err(message.into())
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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
    Ok(path)
}

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().join(format!("extensions-e2e-{}", std::process::id()));
        for dir in ["home", "agent", "cwd"] {
            std::fs::create_dir_all(root.join(dir))?;
        }
        Ok(Self { root })
    }

    fn write(&self, relative: &str, text: &str) -> Result<PathBuf> {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, text)?;
        Ok(path)
    }

    fn settings(&self, settings: &Value) -> Result<()> {
        self.write("agent/settings.json", &serde_json::to_string(settings)?)?;
        Ok(())
    }
}

/// Runs \`pi-rs --mode rpc\`, asks \`get_commands\` once, and returns the answer with stderr.
fn rpc(bin: &Path, scratch: &Scratch, args: &[&str]) -> Result<(Value, String)> {
    let mut child = Command::new(bin)
        .args(args)
        .current_dir(scratch.root.join("cwd"))
        .env_clear()
        .env("HOME", scratch.root.join("home"))
        .env("PI_CODING_AGENT_DIR", scratch.root.join("agent"))
        .env("PATH", "/nonexistent")
        .envs(host_env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    {
        let stdin = child.stdin.as_mut().ok_or("no stdin")?;
        writeln!(stdin, "{}", json!({"type": "get_commands", "id": "1"}))?;
        stdin.flush()?;
    }
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (sender, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if sender.send(line).is_err() {
                return;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut answer = None;
    while Instant::now() < deadline {
        let Ok(line) = lines.recv_timeout(deadline - Instant::now()) else {
            break;
        };
        let value: Value = serde_json::from_str(&line)?;
        if value["type"] == "response" && value["id"] == "1" {
            answer = Some(value);
            break;
        }
    }
    drop(child.stdin.take());
    let exited = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > exited {
            let _ = child.kill();
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr).ok();
    }
    if !status.success() {
        return fail(format!(
            "pi-rs {} exited {status}: {stderr}",
            args.join(" ")
        ));
    }
    match answer {
        Some(value) if value["success"] == true => Ok((value["data"]["commands"].clone(), stderr)),
        Some(value) => fail(format!("get_commands answered {value}")),
        None => fail(format!("no get_commands answer: {stderr}")),
    }
}

/// `PI_EXTENSION_HOST`, when the test harness points at an installed host.
fn host_env() -> Vec<(&'static str, std::ffi::OsString)> {
    std::env::var_os("PI_EXTENSION_HOST")
        .map(|value| vec![("PI_EXTENSION_HOST", value)])
        .unwrap_or_default()
}

fn commands(list: &Value) -> Vec<(String, String)> {
    list.as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    (
                        entry["name"].as_str().unwrap_or_default().to_owned(),
                        entry["description"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn named(list: &Value, name: &str) -> Option<Value> {
    list.as_array()?
        .iter()
        .find(|entry| entry["name"] == name)
        .cloned()
}

fn model(scratch: &Scratch) -> Result<()> {
    let models = json!({"providers": {"fake": {
        "baseUrl": "http://127.0.0.1:9/v1",
        "apiKey": "sk-test",
        "api": "openai-completions",
        "models": [{"id": "fake-test", "contextWindow": 200000, "maxTokens": 8192}],
    }}});
    scratch.write("agent/models.json", &serde_json::to_string(&models)?)?;
    Ok(())
}

const ARGS: &[&str] = &[
    "--provider",
    "fake",
    "--model",
    "fake-test",
    "--no-session",
    "--mode",
    "rpc",
];

fn main() -> Result<()> {
    let bin = binary()?;
    let scratch = Scratch::new()?;
    model(&scratch)?;

    let package = scratch.write(
        "ext-package/package.json",
        &serde_json::to_string(&json!({
            "name": "ext-check-package",
            "type": "module",
            "pi": {"extensions": ["./index.ts"]},
        }))?,
    )?;
    let directory = package.parent().ok_or("no parent")?.to_path_buf();
    scratch.write("ext-package/index.ts", PACKAGE)?;
    let cli = scratch.write("plain/cli.ts", CLI)?;
    let unsupported = scratch.write("unsupported/shortcut.ts", UNSUPPORTED)?;
    let broken = scratch.write("plain/broken.ts", BROKEN)?;

    scratch.settings(&json!({"packages": [directory.to_string_lossy()]}))?;
    let (list, stderr) = rpc(&bin, &scratch, ARGS)?;
    let check = named(&list, "ext-check")
        .ok_or_else(|| format!("no ext-check among {:?}: {stderr}", commands(&list)))?;
    if check["source"] != "extension" {
        return fail(format!("ext-check is {check}, not an extension command"));
    }
    let description = check["description"].as_str().unwrap_or_default();
    if !description.contains("\"required\":[\"text\"]") {
        return fail(format!(
            "ext-check's description is {description:?}, which is not the typebox schema the package computed"
        ));
    }
    let path = check["sourceInfo"]["path"].as_str().unwrap_or_default();
    if !path.ends_with("ext-package/index.ts") {
        return fail(format!("ext-check names {path}"));
    }
    println!("packages setting ok: {description}");

    scratch.settings(&json!({"packages": [directory.to_string_lossy()]}))?;
    let (list, _) = rpc(
        &bin,
        &scratch,
        &[ARGS, &["--extension", &cli.to_string_lossy()]].concat(),
    )?;
    if named(&list, "cli-check").is_none() || named(&list, "ext-check").is_none() {
        return fail(format!(
            "--extension did not add cli-check beside the package: {:?}",
            commands(&list)
        ));
    }
    println!("--extension ok");

    let (list, _) = rpc(&bin, &scratch, &[ARGS, &["--no-extensions"]].concat())?;
    if named(&list, "cli-check").is_some() || named(&list, "ext-check").is_some() {
        return fail(format!(
            "--no-extensions still loaded {:?}",
            commands(&list)
        ));
    }
    println!("--no-extensions ok");

    let (list, _) = rpc(
        &bin,
        &scratch,
        &[
            ARGS,
            &["--no-extensions", "--extension", &cli.to_string_lossy()],
        ]
        .concat(),
    )?;
    if named(&list, "cli-check").is_none() || named(&list, "ext-check").is_some() {
        return fail(format!(
            "--no-extensions should keep --extension and drop the packages: {:?}",
            commands(&list)
        ));
    }
    println!("--no-extensions keeps --extension ok");

    scratch.settings(
        &json!({"extensions": [broken.to_string_lossy(), unsupported.to_string_lossy()]}),
    )?;
    let (list, stderr) = rpc(&bin, &scratch, ARGS)?;
    if !stderr.contains("Warning: Failed to load extension: this extension throws") {
        return fail(format!("the broken extension is not reported: {stderr}"));
    }
    if !stderr.contains("uses registerShortcut, which pi-rs does not implement yet") {
        return fail(format!("registerShortcut is not reported: {stderr}"));
    }
    if !commands(&list).is_empty() {
        return fail(format!(
            "an extension that failed to load still had commands: {:?}",
            commands(&list)
        ));
    }
    println!("load errors ok");

    scratch.settings(&json!({}))?;
    let (list, stderr) = rpc(&bin, &scratch, ARGS)?;
    if !commands(&list).is_empty() || !stderr.is_empty() {
        return fail(format!(
            "with nothing configured pi-rs changed: {:?} {stderr:?}",
            commands(&list)
        ));
    }
    println!("nothing configured ok");

    events(&bin, &scratch)?;
    tools(&bin, &scratch)?;
    cancel(&bin, &scratch)?;
    exec(&bin, &scratch)?;

    std::fs::remove_dir_all(&scratch.root)?;
    println!("extensions e2e ok");
    Ok(())
}

const EVENTS: &str = r#"import fs from "node:fs";
import path from "node:path";

const log = process.env.E2E_LOG;
const note = (line: string): void => {
	fs.appendFileSync(log, line + "\n");
};
const text = (message: any): string =>
	(message.content ?? []).map((block: any) => block.text ?? "").join("");

export default function (pi: any): void {
	pi.on("session_start", (_event: any, ctx: any) => {
		note("session_start");
		note("active:" + pi.getActiveTools().join(","));
		note("all-has-ls:" + pi.getAllTools().some((tool: any) => tool.name === "ls"));
		note("dirname:" + (__dirname === path.dirname(__filename)) + ":" + __filename.endsWith("ext.ts"));
		note("require:" + (require("node:path") === path));
		note("file:" + ctx.sessionManager.getSessionFile());
	});
	pi.on("session_shutdown", (event: any) => note("session_shutdown:" + event.reason));
	pi.on("agent_start", () => note("agent_start"));
	pi.on("agent_end", (_event: any, ctx: any) => {
		note("agent_end");
		const messages = ctx.sessionManager.getBranch().filter((entry: any) => entry.type === "message");
		note("branch:" + messages.map((entry: any) => entry.message.role).join(","));
		note("stored:" + text(messages.at(-1).message));
		note("entries:" + (ctx.sessionManager.getEntries().length === ctx.sessionManager.getBranch().length));
	});
	pi.on("agent_settled", (event: any) => note("agent_settled:" + event.aborted));
	pi.on("message_start", (event: any) => note("message_start:" + event.message.role));
	pi.on("message_update", (event: any) => note("message_update:" + event.message.role));
	pi.on("message_end", (event: any) => {
		note("message_end:" + event.message.role);
		if (event.message.role !== "assistant") return undefined;
		return { message: { ...event.message, content: [{ type: "text", text: "REPLACED BY EXTENSION" }] } };
	});
	pi.on("input", (event: any) => {
		note("input:" + event.text);
		if (event.text.includes("swallow")) return { action: "handled" };
		if (event.text.includes("hello")) return { action: "transform", text: event.text + " (from input)" };
		return undefined;
	});
	pi.registerCommand("e2e-cmd", {
		description: "records its arguments",
		handler: async (args: string) => note("cmd:" + args),
	});
}
"#;

const EXEC: &str = r#"import fs from "node:fs";
import { Type } from "@earendil-works/pi-ai";

const note = (line: string): void => {
	fs.appendFileSync(process.env.E2E_LOG, line + "\n");
};
const BASH = "@BASH@";
const PATH = "@PATH@";

export default function (pi: any): void {
	pi.on("input", async (event: any) => {
		if (event.text.includes("wedge")) await pi.exec(BASH, ["-c", "export PATH=" + PATH + "; sleep 602.5; :"]);
		if (event.text.includes("spin")) {
			await pi.exec(BASH, ["-c", "export PATH=" + PATH + "; sleep 604.5 & :"]);
			for (;;) {}
		}
		if (event.text.includes("background")) {
			const result = await pi.exec(BASH, ["-c", "export PATH=" + PATH + "; sleep 603.5 & :"]);
			note("background:" + result.code + ":" + result.killed);
		}
		return undefined;
	});
	pi.registerTool({
		name: "exec_tree",
		label: "Exec tree",
		description: "runs a tree that ignores SIGTERM and keeps writing, with a timeout",
		parameters: Type.Object({}),
		async execute() {
			const start = Date.now();
			const script = "export PATH=" + PATH + "; : e2e-exec-tree; trap '' TERM; (while :; do echo x; sleep 0.05; done) & wait";
			const result = await pi.exec(BASH, ["-c", script], { timeout: 200 });
			note("exec_tree:" + result.killed + ":" + (Date.now() - start));
			return { content: [{ type: "text", text: "tree done" }], details: {} };
		},
	});
	pi.registerTool({
		name: "exec_hang",
		label: "Exec hang",
		description: "runs a program that outlives the session",
		parameters: Type.Object({}),
		async execute() {
			note("exec_hang:started");
			await pi.exec(BASH, ["-c", "export PATH=" + PATH + "; sleep 601.5; :"]);
			return { content: [{ type: "text", text: "hang done" }], details: {} };
		},
	});
}
"#;

/// An OpenAI-completions server on a real socket: it records each request body and answers with
/// one sentence streamed back in two parts.
fn request_body(stream: &std::net::TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

fn serve_events(bodies: Arc<Mutex<Vec<String>>>) -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let Some(body) = request_body(&stream) else {
                return;
            };
            if let Ok(mut bodies) = bodies.lock() {
                bodies.push(body);
            }
            let head =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            if stream.write_all(head.as_bytes()).is_err() {
                return;
            }
            for (index, part) in ["loop ", "one."].into_iter().enumerate() {
                let delta = if index == 0 {
                    json!({"role": "assistant", "content": part})
                } else {
                    json!({"content": part})
                };
                let chunk = json!({"id": "c1", "choices": [{"index": 0, "delta": delta}]});
                if writeln!(stream, "data: {chunk}\n").is_err() {
                    return;
                }
            }
            let stop = json!({"id": "c1", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]});
            let _ = writeln!(stream, "data: {stop}\n");
            let usage = json!({"id": "c1", "choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 2}});
            let _ = writeln!(stream, "data: {usage}\n");
            let _ = stream.write_all(b"data: [DONE]\n\n");
        }
    });
    Ok(port)
}

/// Events, the answering ones and commands, in one print-mode run: a message is replaced and the
/// input rewritten before the model sees either, and an extension command runs with its arguments.
fn events(bin: &Path, scratch: &Scratch) -> Result<()> {
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let port = serve_events(Arc::clone(&bodies))?;
    let models = json!({"providers": {"fake": {
        "baseUrl": format!("http://127.0.0.1:{port}/v1"),
        "apiKey": "sk-test",
        "api": "openai-completions",
        "models": [{"id": "fake-test", "contextWindow": 200000, "maxTokens": 8192}],
    }}});
    scratch.write("agent/models.json", &serde_json::to_string(&models)?)?;
    let extension = scratch.write("events/ext.ts", EVENTS)?;
    scratch.write(
        "agent/settings.json",
        &serde_json::to_string(&json!({"extensions": [extension.to_string_lossy()]}))?,
    )?;
    let log = scratch.root.join("events.log");
    let mut child = Command::new(bin)
        .args([
            "-p",
            "hello",
            "-p",
            "again",
            "-p",
            "/e2e-cmd one two",
            "--provider",
            "fake",
            "--model",
            "fake-test",
            "--no-session",
        ])
        .current_dir(scratch.root.join("cwd"))
        .env_clear()
        .env("HOME", scratch.root.join("home"))
        .env("PI_CODING_AGENT_DIR", scratch.root.join("agent"))
        .env("PATH", "/nonexistent")
        .env("E2E_LOG", &log)
        .envs(host_env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return fail("pi-rs did not finish the print-mode run".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_string(&mut stdout).ok();
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr).ok();
    }
    if !status.success() {
        return fail(format!("pi-rs exited {status}: {stderr}"));
    }
    let seen = std::fs::read_to_string(&log).unwrap_or_default();
    for wanted in [
        "session_start",
        "agent_start",
        "message_start:assistant",
        "message_update:assistant",
        "message_end:assistant",
        "agent_end",
        "agent_settled:false",
        "input:hello",
        "input:again",
        "cmd:one two",
        "session_shutdown:quit",
        "active:read,bash,edit,write",
        "all-has-ls:true",
        "dirname:true:true",
        "require:true",
        "file:undefined",
        "branch:system,user,assistant",
        "branch:system,user,assistant,user,assistant",
        "stored:REPLACED BY EXTENSION",
        "entries:true",
    ] {
        if !seen.lines().any(|line| line == wanted) {
            return fail(format!(
                "no {wanted:?} in the extension log:\n{seen}\nstderr:\n{stderr}"
            ));
        }
    }
    println!("events ok: {}", seen.lines().count());
    let recorded = bodies.lock().map(|b| b.clone()).unwrap_or_default();
    let Some(first) = recorded.first() else {
        return fail("the model was never called".to_owned());
    };
    let second = recorded.get(1).ok_or("the model was called once")?;
    if !first.contains("hello (from input)") {
        return fail(format!(
            "the input event did not rewrite the prompt: {first}"
        ));
    }
    if first.contains("REPLACED BY EXTENSION") {
        return fail(format!(
            "the first request already held the replacement: {first}"
        ));
    }
    if !second.contains("REPLACED BY EXTENSION") {
        return fail(format!(
            "the replaced message is not in the next request: {second}"
        ));
    }
    if second.contains("loop one.") && stdout.trim().is_empty() {
        return fail("print mode printed nothing for the replaced reply".to_owned());
    }
    println!("message_end replacement and input transform ok");
    Ok(())
}

const TOOLS: &str = r#"import fs from "node:fs";
import { Type } from "@earendil-works/pi-ai";
import { defineTool } from "@earendil-works/pi-coding-agent";

const note = (line: string): void => {
	fs.appendFileSync(process.env.E2E_LOG, line + "\n");
};

export default function (pi: any): void {
	pi.registerTool(
		defineTool({
			name: "load_echo",
			label: "Load echo",
			description: "registered while loading",
			parameters: Type.Object({ text: Type.String() }),
			async execute(_id: string, params: any) {
				return { content: [{ type: "text", text: "load: " + params.text }], details: {} };
			},
		}),
	);
	pi.registerTool({
		name: "wait_abort",
		label: "Wait",
		description: "waits until it is aborted",
		parameters: Type.Object({}),
		execute(_id: string, _params: any, signal: AbortSignal) {
			note("wait_abort:started");
			return new Promise((_resolve, reject) => {
				signal.addEventListener("abort", () => {
					note("wait_abort:aborted");
					reject(new Error("aborted"));
				});
			});
		},
	});
	pi.on("session_start", () => {
		pi.registerTool({
			name: "late_echo",
			label: "Late echo",
			description: "registered at session_start",
			parameters: Type.Object({ text: Type.String() }),
			async execute(id: string, params: any, _signal: AbortSignal, onUpdate: any, ctx: any) {
				onUpdate({ content: [{ type: "text", text: "partial " }], details: { step: 1 } });
				onUpdate({ content: [{ type: "text", text: "partial one" }], details: { step: 2 } });
				note("late_echo:" + id + ":" + params.text + ":" + (ctx.cwd.length > 0));
				return { content: [{ type: "text", text: "echo: " + params.text }], details: { echoed: params.text } };
			},
		});
		note("active:" + pi.getActiveTools().join(","));
	});
}
"#;

enum Turn {
    Text(&'static str),
    Tool(&'static str, Value),
}

fn serve_script(turns: Vec<Turn>, bodies: Arc<Mutex<Vec<String>>>) -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for (index, stream) in listener.incoming().enumerate() {
            let Ok(mut stream) = stream else { return };
            let Some(body) = request_body(&stream) else {
                return;
            };
            if let Ok(mut bodies) = bodies.lock() {
                bodies.push(body);
            }
            let head =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            if stream.write_all(head.as_bytes()).is_err() {
                return;
            }
            let (delta, finish) = match turns.get(index) {
                Some(Turn::Tool(name, args)) => (
                    json!({"role": "assistant", "tool_calls": [{
                        "index": 0,
                        "id": format!("call_{}", index + 1),
                        "type": "function",
                        "function": {"name": name, "arguments": args.to_string()},
                    }]}),
                    "tool_calls",
                ),
                Some(Turn::Text(text)) => (json!({"role": "assistant", "content": text}), "stop"),
                None => (
                    json!({"role": "assistant", "content": "unscripted"}),
                    "stop",
                ),
            };
            for chunk in [
                json!({"id": "c1", "choices": [{"index": 0, "delta": delta}]}),
                json!({"id": "c1", "choices": [{"index": 0, "delta": {}, "finish_reason": finish}]}),
                json!({"id": "c1", "choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 2}}),
            ] {
                if writeln!(stream, "data: {chunk}\n").is_err() {
                    return;
                }
            }
            let _ = stream.write_all(b"data: [DONE]\n\n");
        }
    });
    Ok(port)
}

fn scripted(
    scratch: &Scratch,
    source: &str,
    turns: Vec<Turn>,
) -> Result<(Arc<Mutex<Vec<String>>>, PathBuf)> {
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let port = serve_script(turns, Arc::clone(&bodies))?;
    let models = json!({"providers": {"fake": {
        "baseUrl": format!("http://127.0.0.1:{port}/v1"),
        "apiKey": "sk-test",
        "api": "openai-completions",
        "models": [{"id": "fake-test", "contextWindow": 200000, "maxTokens": 8192}],
    }}});
    scratch.write("agent/models.json", &serde_json::to_string(&models)?)?;
    let extension = scratch.write("tools/ext.ts", source)?;
    scratch.settings(&json!({"extensions": [extension.to_string_lossy()]}))?;
    let log = scratch.root.join("tools.log");
    let _ = std::fs::remove_file(&log);
    Ok((bodies, log))
}

fn launch(bin: &Path, scratch: &Scratch, log: &Path, args: &[&str]) -> Result<std::process::Child> {
    Ok(Command::new(bin)
        .args(args)
        .args(["--provider", "fake", "--model", "fake-test", "--no-session"])
        .current_dir(scratch.root.join("cwd"))
        .env_clear()
        .env("HOME", scratch.root.join("home"))
        .env("PI_CODING_AGENT_DIR", scratch.root.join("agent"))
        .env("PATH", "/nonexistent")
        .env("E2E_LOG", log)
        .envs(host_env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?)
}

fn tools(bin: &Path, scratch: &Scratch) -> Result<()> {
    let (bodies, log) = scripted(
        scratch,
        TOOLS,
        vec![
            Turn::Tool("late_echo", json!({"text": "hi"})),
            Turn::Text("done"),
        ],
    )?;
    let mut child = launch(bin, scratch, &log, &["--mode", "json", "-p", "echo hi"])?;
    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return fail("pi-rs did not finish the tool run".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_string(&mut stdout).ok();
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr).ok();
    }
    if !status.success() {
        return fail(format!("pi-rs exited {status}: {stderr}"));
    }
    let events: Vec<Value> = stdout
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let of = |kind: &str| {
        events
            .iter()
            .filter(|event| event["type"] == kind && event["toolName"] == "late_echo")
            .cloned()
            .collect::<Vec<_>>()
    };
    if of("tool_execution_start").is_empty() {
        return fail(format!("late_echo never started:\n{stdout}\n{stderr}"));
    }
    if !of("tool_execution_update")
        .iter()
        .any(|event| event["partialResult"]["details"]["step"] == 2)
    {
        return fail(format!("late_echo's progress never arrived:\n{stdout}"));
    }
    let end = of("tool_execution_end");
    let Some(end) = end.first() else {
        return fail(format!("late_echo never ended:\n{stdout}"));
    };
    if end["result"]["content"][0]["text"] != "echo: hi"
        || end["result"]["details"]["echoed"] != "hi"
        || end["isError"] == true
    {
        return fail(format!("late_echo ended with {end}"));
    }
    let seen = std::fs::read_to_string(&log).unwrap_or_default();
    if !seen.lines().any(|line| line == "late_echo:call_1:hi:true") {
        return fail(format!(
            "late_echo did not run in the child:\n{seen}\n{stderr}"
        ));
    }
    if !seen.lines().any(|line| {
        line.starts_with("active:") && line.contains("load_echo") && line.contains("late_echo")
    }) {
        return fail(format!("the extension tools are not active:\n{seen}"));
    }
    let recorded = bodies.lock().map(|b| b.clone()).unwrap_or_default();
    let first = recorded.first().ok_or("the model was never called")?;
    if !first.contains("\"late_echo\"") || !first.contains("\"load_echo\"") {
        return fail(format!(
            "the model was not offered both extension tools: {first}"
        ));
    }
    let second = recorded.get(1).ok_or("the model was called once")?;
    if !second.contains("echo: hi") {
        return fail(format!(
            "the tool result is not in the next request: {second}"
        ));
    }
    println!("extension tools ok");
    Ok(())
}

fn cancel(bin: &Path, scratch: &Scratch) -> Result<()> {
    let (_bodies, log) = scripted(
        scratch,
        TOOLS,
        vec![Turn::Tool("wait_abort", json!({})), Turn::Text("after")],
    )?;
    let mut child = launch(bin, scratch, &log, &["--mode", "rpc"])?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (sender, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if sender.send(line).is_err() {
                return;
            }
        }
    });
    writeln!(
        stdin,
        "{}",
        json!({"type": "prompt", "id": "p", "message": "wait"})
    )?;
    stdin.flush()?;
    let wait_for = |wanted: &dyn Fn(&Value) -> bool, what: &str| -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            let Ok(line) = lines.recv_timeout(deadline - Instant::now()) else {
                break;
            };
            if serde_json::from_str::<Value>(&line).is_ok_and(|value| wanted(&value)) {
                return Ok(());
            }
        }
        fail(format!("no {what} from pi-rs"))
    };
    wait_for(
        &|value| value["type"] == "tool_execution_start" && value["toolName"] == "wait_abort",
        "tool_execution_start",
    )?;
    let started = Instant::now() + Duration::from_secs(30);
    while !std::fs::read_to_string(&log)
        .unwrap_or_default()
        .contains("wait_abort:started")
    {
        if Instant::now() > started {
            return fail("wait_abort never ran in the child".to_owned());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    writeln!(stdin, "{}", json!({"type": "abort", "id": "a"}))?;
    stdin.flush()?;
    wait_for(
        &|value| value["type"] == "agent_end",
        "agent_end after the abort",
    )?;
    drop(stdin);
    let exited = Instant::now() + Duration::from_secs(30);
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() > exited {
            let _ = child.kill();
            return fail("pi-rs did not exit after the abort".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let seen = std::fs::read_to_string(&log).unwrap_or_default();
    if !seen.lines().any(|line| line == "wait_abort:aborted") {
        return fail(format!(
            "the abort never reached the tool's signal:\n{seen}"
        ));
    }
    println!("extension tool cancellation ok");
    Ok(())
}

fn marked(marker: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
        })
        .filter_map(|entry| std::fs::read(entry.path().join("cmdline")).ok())
        .map(|cmdline| String::from_utf8_lossy(&cmdline).replace('\0', " "))
        .filter(|cmdline| cmdline.contains(marker))
        .collect()
}

fn gone(marker: &str, within: Duration) -> Result<()> {
    let deadline = Instant::now() + within;
    loop {
        let left = marked(marker);
        if left.is_empty() {
            return Ok(());
        }
        if Instant::now() > deadline {
            return fail(format!("{marker} outlived its exec: {left:?}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn exec_source() -> Result<String> {
    let path = std::env::var("PATH")?;
    let bash = path
        .split(':')
        .map(|dir| Path::new(dir).join("bash"))
        .find(|candidate| executable(candidate))
        .ok_or("bash not found on PATH")?;
    Ok(EXEC
        .replace("@BASH@", &bash.to_string_lossy())
        .replace("@PATH@", &path))
}

fn exec(bin: &Path, scratch: &Scratch) -> Result<()> {
    let source = exec_source()?;
    let (_bodies, log) = scripted(
        scratch,
        &source,
        vec![Turn::Tool("exec_tree", json!({})), Turn::Text("done")],
    )?;
    let mut child = launch(bin, scratch, &log, &["--mode", "json", "-p", "run it"])?;
    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait()?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            return fail("pi-rs never finished a run whose exec timed out".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let seen = std::fs::read_to_string(&log).unwrap_or_default();
    let Some(ms) = seen
        .lines()
        .find_map(|line| line.strip_prefix("exec_tree:true:"))
        .and_then(|ms| ms.parse::<u64>().ok())
    else {
        return fail(format!("exec_tree was not killed:\n{seen}"));
    };
    if !(4500..15000).contains(&ms) {
        return fail(format!(
            "exec_tree took {ms} ms, not SIGTERM then SIGKILL 5 s on"
        ));
    }
    gone("e2e-exec-tree", Duration::from_secs(2))?;

    for how in ["quit", "killed"] {
        let (_bodies, log) = scripted(
            scratch,
            &source,
            vec![Turn::Tool("exec_hang", json!({})), Turn::Text("done")],
        )?;
        let mut child = launch(bin, scratch, &log, &["--mode", "rpc"])?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if sender.send(line).is_err() {
                    return;
                }
            }
        });
        writeln!(
            stdin,
            "{}",
            json!({"type": "prompt", "id": "p", "message": "hang"})
        )?;
        stdin.flush()?;
        let started = Instant::now() + Duration::from_secs(60);
        while marked("601.5").is_empty() {
            if Instant::now() > started {
                let _ = child.kill();
                return fail(format!("exec_hang never started its program ({how})"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if how == "killed" {
            child.kill()?;
        } else {
            writeln!(stdin, "{}", json!({"type": "abort", "id": "a"}))?;
            stdin.flush()?;
            let ended = Instant::now() + Duration::from_secs(60);
            loop {
                let Ok(line) = lines.recv_timeout(ended.saturating_duration_since(Instant::now()))
                else {
                    let _ = child.kill();
                    return fail("no agent_end after aborting exec_hang".to_owned());
                };
                if serde_json::from_str::<Value>(&line).is_ok_and(|v| v["type"] == "agent_end") {
                    break;
                }
            }
        }
        drop(stdin);
        let exited = Instant::now() + Duration::from_secs(30);
        while child.try_wait()?.is_none() {
            if Instant::now() > exited {
                let _ = child.kill();
                return fail(format!("pi-rs did not exit ({how})"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        gone("601.5", Duration::from_secs(5))?;
    }
    let (_bodies, log) = scripted(scratch, &source, vec![Turn::Text("done")])?;
    let mut child = launch(bin, scratch, &log, &["--mode", "json", "-p", "wedge"])?;
    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait()?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            return fail("pi-rs never gave up on a wedged input handler".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    gone("602.5", Duration::from_secs(5))?;

    let (_bodies, log) = scripted(scratch, &source, vec![Turn::Text("done")])?;
    let mut child = launch(bin, scratch, &log, &["--mode", "json", "-p", "background"])?;
    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait()?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            return fail(
                "pi-rs never finished a run that left a program in the background".to_owned(),
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let seen = std::fs::read_to_string(&log).unwrap_or_default();
    if !seen.lines().any(|line| line == "background:0:false") {
        return fail(format!(
            "the backgrounding exec did not return at once:\n{seen}"
        ));
    }
    gone("603.5", Duration::from_secs(5))?;

    let (_bodies, log) = scripted(scratch, &source, vec![Turn::Text("done")])?;
    let mut child = launch(bin, scratch, &log, &["--mode", "json", "-p", "spin"])?;
    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait()?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            return fail("pi-rs never gave up on a spinning input handler".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    gone("604.5", Duration::from_secs(5))?;
    println!(
        "extension exec ok: timeout kills the tree; quitting, killing pi-rs, a waiting or spinning handler, or a background job leaves no program"
    );
    Ok(())
}
