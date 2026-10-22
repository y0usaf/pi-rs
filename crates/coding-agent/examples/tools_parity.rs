//! Runs the tool cases of `parity/tools.json` and prints one JSON line per case, for
//! `parity/run.sh` to compare with upstream's tools on the same cases.
//!
//! Usage: `tools_parity CASES.json WORKDIR`

use pi_coding_agent::tools::{self, Outcome, ToolContext};
use pi_durable::ai::types::UserBlock;
use pi_durable::cancel::CancelToken;
use serde_json::{Value, json};
use std::path::Path;

fn setup(case: &Value, dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    if let Some(files) = case["files"].as_object() {
        for (name, text) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text.as_str().unwrap()).unwrap();
        }
    }
    if let Some(gen_files) = case["gen"].as_object() {
        for (name, spec) in gen_files {
            let lines = spec["lines"].as_u64().unwrap();
            let width = spec["width"].as_u64().unwrap() as usize;
            let body: Vec<String> = (0..lines)
                .map(|i| format!("{i:0>width$}").replace('0', "x"))
                .collect();
            std::fs::write(dir.join(name), body.join("\n")).unwrap();
        }
    }
    if let Some(b64) = case["b64"].as_object() {
        for (name, data) in b64 {
            let out = std::process::Command::new("base64")
                .arg("-d")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut c| {
                    use std::io::Write;
                    c.stdin
                        .take()
                        .unwrap()
                        .write_all(data.as_str().unwrap().as_bytes())?;
                    c.wait_with_output()
                })
                .unwrap();
            std::fs::write(dir.join(name), out.stdout).unwrap();
        }
    }
    if let Some(dirs) = case["mkdir"].as_array() {
        for d in dirs {
            std::fs::create_dir_all(dir.join(d.as_str().unwrap())).unwrap();
        }
    }
}

fn blocks(content: &[UserBlock]) -> Value {
    Value::Array(
        content
            .iter()
            .map(|b| match b {
                UserBlock::Text(t) => json!({"type": "text", "text": t.text}),
                UserBlock::Image(i) => {
                    json!({"type": "image", "mimeType": i.mime_type, "bytes": i.data.len()})
                }
            })
            .collect(),
    )
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cases: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(&args[1]).unwrap()).unwrap();
    let root = Path::new(&args[2]);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let dir = root.join(name);
        setup(&case, &dir);
        let cwd = dir.to_string_lossy().into_owned();
        let spec = tools::spec(case["tool"].as_str().unwrap()).unwrap();
        let cancel = CancelToken::new();
        let mut ctx = ToolContext {
            cwd: &cwd,
            cancel: &cancel,
            model_takes_images: true,
            shell_path: None,
            env: &[],
            on_update: None,
        };
        let outcome = tools::run(
            &spec,
            case["args"].as_object().cloned().unwrap_or_default(),
            &mut ctx,
        );
        let mut result = match outcome {
            Outcome::Done(out) => {
                json!({"content": blocks(&out.content), "details": out.details, "isError": out.is_error})
            }
            Outcome::Threw(message) => json!({"threw": message}),
        };
        if let Some(show) = case["show"].as_array() {
            let files: serde_json::Map<String, Value> = show
                .iter()
                .map(|p| {
                    let p = p.as_str().unwrap();
                    (
                        p.to_owned(),
                        json!(std::fs::read_to_string(dir.join(p)).unwrap_or_default()),
                    )
                })
                .collect();
            result["files"] = Value::Object(files);
        }
        println!("{}", json!({"name": name, "dir": cwd, "result": result}));
    }
}
