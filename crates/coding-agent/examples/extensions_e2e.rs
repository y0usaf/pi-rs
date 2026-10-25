use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
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

const TOOLS: &str = r#"import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI): void {
	pi.registerTool({ name: "check_tool", description: "a tool", parameters: {} });
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
    let tools = scratch.write("tools/tool.ts", TOOLS)?;
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

    scratch
        .settings(&json!({"extensions": [broken.to_string_lossy(), tools.to_string_lossy()]}))?;
    let (list, stderr) = rpc(&bin, &scratch, ARGS)?;
    if !stderr.contains("Warning: Failed to load extension: this extension throws") {
        return fail(format!("the broken extension is not reported: {stderr}"));
    }
    if !stderr.contains("uses registerTool, which pi-rs does not implement yet") {
        return fail(format!("registerTool is not reported: {stderr}"));
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

    std::fs::remove_dir_all(&scratch.root)?;
    println!("extensions e2e ok");
    Ok(())
}
