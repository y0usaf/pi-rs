//! Drives `pi-rs --codemode-vm` end to end: the child gets one start message on stdin and reports
//! the script's output and result as JSON lines on stdout.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// Run one script in a fresh child and return the messages it reported.
fn vm(bin: &Path, start: &Value) -> Result<Vec<Value>> {
    let mut child = Command::new(bin)
        .arg("--codemode-vm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    writeln!(stdin, "{start}")?;
    drop(stdin);
    let out = child.wait_with_output()?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        return fail(format!(
            "pi-rs --codemode-vm exited {}: {stderr}",
            out.status
        ));
    }
    let mut lines = Vec::new();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        match serde_json::from_str(line) {
            Ok(value) => lines.push(value),
            Err(error) => return fail(format!("stdout line is not JSON ({error}): {line}")),
        }
    }
    Ok(lines)
}

fn start(code: &str) -> Value {
    json!({
        "code": code,
        "tools": [],
        "globals": [],
        "memoryLimitBytes": 268435456,
        "store": {},
    })
}

fn need<'a>(lines: &'a [Value], what: &str, found: impl Fn(&Value) -> bool) -> Result<&'a Value> {
    lines
        .iter()
        .find(|line| found(line))
        .ok_or_else(|| format!("no {what} among {} messages: {lines:#?}", lines.len()).into())
}

fn done(lines: &[Value]) -> Result<&Value> {
    need(lines, "done message", |line| line["type"] == "done")
}

fn output(lines: &[Value]) -> Vec<(String, String)> {
    lines
        .iter()
        .filter(|line| line["type"] == "output")
        .map(|line| {
            let item = &line["item"];
            let kind = if item["console"] == true {
                "console"
            } else {
                item["type"].as_str().unwrap_or_default()
            };
            (
                kind.to_owned(),
                item["text"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

fn output_and_return(bin: &Path) -> Result<()> {
    let lines = vm(
        bin,
        &start(
            r#"text("hello");
console.log("logged", { a: 1 });
return { total: 42 };"#,
        ),
    )?;
    let items = output(&lines);
    let expected = [
        ("text".to_owned(), "hello".to_owned()),
        ("console".to_owned(), r#"logged {"a":1}"#.to_owned()),
    ];
    if items != expected {
        return fail(format!("output reads {items:?}, expected {expected:?}"));
    }
    let done = done(&lines)?;
    if done["ok"] != true {
        return fail(format!("the script failed: {done:#?}"));
    }
    if done["value"] != json!(r#"{"total":42}"#) {
        return fail(format!("the returned value is {:#?}", done["value"]));
    }
    if done["writes"] != json!("[]") {
        return fail(format!("store writes are {:#?}", done["writes"]));
    }
    let last = lines.last().expect("lines");
    if last["type"] != "done" {
        return fail(format!("{last:#?} came after done"));
    }
    Ok(())
}

fn thrown_error(bin: &Path) -> Result<()> {
    let lines = vm(bin, &start("text(\"before\");\nthrow new Error(\"boom\");"))?;
    let items = output(&lines);
    if items != [("text".to_owned(), "before".to_owned())] {
        return fail(format!("output reads {items:?}"));
    }
    let done = done(&lines)?;
    if done["ok"] != false {
        return fail(format!("the script did not fail: {done:#?}"));
    }
    let error: Value = serde_json::from_str(done["error"].as_str().unwrap_or_default())?;
    let message = error["message"].as_str().unwrap_or_default();
    if message != "boom" {
        return fail(format!("the error message is {message:?}: {done:#?}"));
    }
    let stack = error["stack"].as_str().unwrap_or_default();
    if !stack.contains("codemode.js") || stack.contains("codemode-prelude.js") {
        return fail(format!("the stack reads {stack:?}"));
    }
    Ok(())
}

fn syntax_error(bin: &Path) -> Result<()> {
    let lines = vm(bin, &start("this is not javascript"))?;
    let done = done(&lines)?;
    if done["ok"] != false || done.get("value").is_some() {
        return fail(format!("a syntax error did not fail cleanly: {done:#?}"));
    }
    let error: Value = serde_json::from_str(done["error"].as_str().unwrap_or_default())?;
    if error["name"] != json!("SyntaxError") {
        return fail(format!("the syntax error reads {error:#?}: {done:#?}"));
    }
    let stack = error["stack"].as_str().unwrap_or_default();
    if !stack.starts_with("SyntaxError: ") || !stack.contains("codemode.js:1") {
        return fail(format!(
            "the syntax error's stack reads {stack:?}: {done:#?}"
        ));
    }
    Ok(())
}

fn main() -> Result<()> {
    let bin = binary()?;
    output_and_return(&bin)?;
    thrown_error(&bin)?;
    syntax_error(&bin)?;
    println!("codemode vm ok");
    Ok(())
}
