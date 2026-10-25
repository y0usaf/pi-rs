//! The `codemode` tool: the model writes JavaScript that calls other tools
//! (`packages/coding-agent/src/extensions/codemode/tool.ts` and `execute.ts`).
//!
//! A script runs in the QuickJS child from [pi_codemode] and reaches the agent's tools through
//! [pi_durable::harness::tool::nested_call], so every nested call passes the gates a model-issued
//! one does. Only the script's output reaches the model; nested results do not.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use pi_codemode::host::{CallStatus, ErrorKind, ExecuteOptions, ScriptResult};
use pi_codemode::{DeclaredTool, OutputItem};
use pi_durable::ai::types::{ImageContent, ImageType, UserBlock};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::tool::nested_call;
use pi_durable::harness::types::{
    OutputLimitsSpec, ToolExecute, ToolExecutionApi, ToolExecutionResult, ToolRegistration,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::output_files;
use crate::tools::truncate::format_size;
use crate::tools::{PromptContribution, text_block};

mod declarations;

pub const TOOL_NAME: &str = "codemode";

/// Custom entry type holding one script's `store()` writes.
pub const STORE_ENTRY_TYPE: &str = "codemode-store";

/// Heap limit for the QuickJS VM: without one a runaway script can grow to wasm32's 4 GiB.
const MEMORY_LIMIT_BYTES: usize = 256 * 1024 * 1024;
/// Default for `codemode.inlineBudget`.
const DEFAULT_INLINE_BUDGET: f64 = 3000.0;
/// Default token budget for script output.
const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 10_000;
/// Characters per token when estimating.
const CHARS_PER_TOKEN: usize = 4;
const ARGS_PREVIEW_CHARS: usize = 200;
const ERROR_PREVIEW_CHARS: usize = 500;
/// Allow one replacement while a cancelled child is still exiting.
const DEFAULT_MAX_WORKERS: usize = 2;
/// How long a script waits for a free worker before failing.
const WORKER_WAIT_MS: u64 = 60_000;

pub fn prompt_contribution() -> PromptContribution {
    PromptContribution {
        snippet: "Run JavaScript that calls other tools",
        guidelines: &[
            "Use codemode to batch independent tool calls (Promise.allSettled), chain them, or filter large output, instead of many separate calls.",
        ],
    }
}

/// `codemodeSchema`: one string, the script.
pub fn parameters() -> Value {
    json!({
        "type": "object",
        "required": ["code"],
        "properties": {
            "code": {"type": "string", "description": "Raw JavaScript source."}
        }
    })
}

const DESCRIPTION_INTRO: &str = r#"Run JavaScript that calls other tools. The input is raw JavaScript (not JSON, no code fence), run as an async function body in a QuickJS sandbox: top-level `await` and `return` work. No Node, file system, network, or timers.
- `await tools.<name>({ ...args })` resolves to a string, or an object if the tool's declaration says so, and rejects with an Error on failure. A tool whose declaration returns an object resolves on failure too when it returned that object; check its fields. Calls still running when the script ends are cancelled.
- Optional first line: `// @options: {"max_output_tokens": 10000, "timeout_ms": 60000}`"#;

const GLOBALS: &str = "Globals:
- `text(value)`, `image(dataUrlOrImageBlock)`, `console.log(...)`, and top-level `return` add output; `exit()` ends the script. With several text items, each starts with a `==> text N/M <==` line, and `console` lines follow the other output in one `<console_output>` block. `image()` also saves the image to a temp file and the result names its path.
- `store(key, value)` and `load(key)` keep JSON values across codemode calls.
- `ALL_TOOLS` lists every tool a script can call.";

/// The model-facing description. Without tools there is no list, so this is what a registered
/// codemode tool says before a loadout gives it the script's tools.
pub fn description() -> String {
    format!("{DESCRIPTION_INTRO}\n\n{GLOBALS}")
}

#[derive(Deserialize)]
struct Args {
    code: String,
}

/// The codemode tool as a durable registration. The description is the loadout's.
pub fn registration(description: String) -> ToolRegistration {
    let execute: ToolExecute = Arc::new(|args, api, cancel| {
        let args: Args = serde_json::from_value(args)
            .map_err(|error| print_error(format!("Invalid arguments: {error}")))?;
        run(&args.code, api, cancel)
    });
    ToolRegistration {
        tool: pi_durable::ai::types::Tool {
            name: TOOL_NAME.to_owned(),
            description,
            parameters: parameters(),
            constrained_sampling: None,
        },
        replay: None,
        execution_mode: None,
        // The script's output is formatted and truncated here, as upstream does, so the
        // harness's own bound is lifted.
        output_limits: Some(OutputLimitsSpec {
            max_bytes: Some(usize::MAX),
            max_lines: Some(usize::MAX),
            retain: None,
        }),
        prepare_arguments: None,
        exposure: pi_durable::harness::types::ToolExposure::ModelOnly,
        hidden: false,
        execute,
    }
}

fn print_error(message: String) -> pi_durable::errors::Error {
    pi_durable::errors::Error::Invalid(message)
}

fn text_of(result: &ToolExecutionResult) -> String {
    result
        .content
        .as_ref()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| match block {
                    UserBlock::Text(text) => Some(text.text.as_str()),
                    UserBlock::Image(_) => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn preview(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max_chars.saturating_sub(3)).collect();
    cut.push_str("...");
    cut
}

fn preview_args(args: Option<&Value>) -> String {
    match args {
        None => String::new(),
        Some(args) => preview(
            &serde_json::to_string(args).unwrap_or_default(),
            ARGS_PREVIEW_CHARS,
        ),
    }
}

/// Run one script, and turn what the sandbox reported into the model's result.
fn run(
    code: &str,
    api: &dyn ToolExecutionApi,
    cancel: &CancelToken,
) -> pi_durable::errors::Result<ToolExecutionResult> {
    let started = Instant::now();
    let parsed = match pi_codemode::parse_source(code) {
        Ok(parsed) => parsed,
        Err(message) => {
            return Ok(ToolExecutionResult {
                content: Some(vec![text_block(message)]),
                is_error: Some(true),
                details: Some(json!({"calls": []})),
                ..ToolExecutionResult::default()
            });
        }
    };
    let callable: Vec<DeclaredTool> = api
        .registry()
        .tools()
        .filter(|(_, tool)| tool.tool.name != TOOL_NAME && tool.exposure.callable(true))
        .map(|(_, tool)| {
            DeclaredTool::new(
                &tool.tool.name,
                &declarations::render_sample(
                    &tool.tool.name,
                    Some(&tool.tool.description),
                    Some(&tool.tool.parameters),
                    Some(&output_schema(&tool.tool)),
                ),
            )
        })
        .collect();
    let deadline = parsed.options.timeout_ms;
    let waited = Instant::now();
    let worker = match Workers::acquire(deadline, cancel) {
        Ok(worker) => worker,
        Err(refusal) => {
            let wall = (Instant::now() - started).as_secs_f64();
            let error = refusal.message(max_workers());
            return Ok(failed_result(
                wall,
                &error,
                "The script did not start.",
                &[],
            ));
        }
    };
    let remaining = deadline.map(|ms| {
        ms.saturating_sub(waited.elapsed().as_millis() as u64)
            .max(1)
    });

    let mut calls: Vec<CallRecord> = Vec::new();
    let mut index = 0usize;
    let cancelled = || cancel.is_cancelled();
    let mut execute =
        |name: &str, args: Option<&Value>| -> std::result::Result<Option<Value>, String> {
            index += 1;
            let mut record = CallRecord {
                id: format!("{}/{index}", api.call_id()),
                name: name.to_owned(),
                args: preview_args(args),
                status: CallStatus::Cancelled,
                duration_ms: 0.0,
                error: None,
            };
            let call_started = Instant::now();
            let arguments = match args {
                None => Map::new(),
                Some(Value::Object(map)) => map.clone(),
                Some(_) => return Err("Tool arguments must be an object".to_owned()),
            };
            let outcome = nested_call(api, &record.id, name, arguments, cancel);
            record.duration_ms = call_started.elapsed().as_secs_f64() * 1000.0;
            let outcome = match outcome {
                Ok(outcome) if outcome.is_error == Some(true) => {
                    record.status = CallStatus::Error;
                    let text = text_of(&outcome);
                    let text = if text.is_empty() {
                        format!("Tool \"{name}\" failed")
                    } else {
                        text
                    };
                    record.error = Some(preview(&text, ERROR_PREVIEW_CHARS));
                    Err(text)
                }
                Ok(outcome) => {
                    record.status = CallStatus::Ok;
                    Ok(Some(Value::String(text_of(&outcome))))
                }
                Err(error) => {
                    record.status = CallStatus::Error;
                    record.error = Some(preview(&error.to_string(), ERROR_PREVIEW_CHARS));
                    Err(error.to_string())
                }
            };
            calls.push(record);
            outcome
        };

    let store = read_store(api, cancel)?;
    let result = pi_codemode::execute(
        &vm_path(),
        &parsed.code,
        &callable,
        &[],
        &store,
        ExecuteOptions {
            timeout_ms: remaining,
            memory_limit_bytes: Some(MEMORY_LIMIT_BYTES),
            cancelled: Some(&cancelled),
            execute: &mut execute,
        },
    );
    drop(worker);
    if cancel.is_cancelled() {
        return Err(pi_durable::errors::Error::Aborted);
    }
    let writes = &result.store_writes;
    if result.ok && !writes.is_empty() {
        api.append_custom_entry(
            STORE_ENTRY_TYPE,
            json!({"set": writes.set, "delete": writes.delete}),
            cancel,
        )?;
    }
    Ok(finish(
        &result,
        &calls,
        started,
        parsed
            .options
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS),
    ))
}

/// One script's `store()` writes: the shape of a `codemode-store` entry's data.
#[derive(Deserialize)]
struct StoreEntry {
    #[serde(default)]
    set: Map<String, Value>,
    #[serde(default)]
    delete: Vec<String>,
}

/// What `load()` returns: the `codemode-store` entries on the branch, applied from the root, as the
/// JSON text the sandbox keeps each value in.
fn read_store(
    api: &dyn ToolExecutionApi,
    cancel: &CancelToken,
) -> pi_durable::errors::Result<BTreeMap<String, String>> {
    let mut store: BTreeMap<String, Value> = BTreeMap::new();
    for entry in api.branch_entries(cancel)? {
        if entry.kind != STORE_ENTRY_TYPE {
            continue;
        }
        let Some(Ok(data)) = entry.data.map(serde_json::from_value::<StoreEntry>) else {
            continue;
        };
        for key in data.delete {
            store.remove(&key);
        }
        for (key, value) in data.set {
            store.insert(key, value);
        }
    }
    Ok(store
        .into_iter()
        .map(|(key, value)| (key, value.to_string()))
        .collect())
}

/// The pi-rs binary, which carries the hidden `--codemode-vm` mode.
fn vm_path() -> std::path::PathBuf {
    match std::env::var("PI_CODEMODE_VM_BIN") {
        Ok(path) if !path.is_empty() => std::path::PathBuf::from(path),
        _ => std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("pi-rs")),
    }
}

struct CallRecord {
    id: String,
    name: String,
    args: String,
    status: CallStatus,
    duration_ms: f64,
    error: Option<String>,
}

impl CallRecord {
    fn json(&self) -> Value {
        let mut value = json!({
            "id": self.id,
            "name": self.name,
            "args": self.args,
            "status": self.status.name(),
            "durationMs": self.duration_ms.round(),
        });
        if let (Some(error), Some(object)) = (&self.error, value.as_object_mut()) {
            object.insert("error".to_owned(), json!(error));
        }
        value
    }
}

fn finish(
    result: &ScriptResult,
    calls: &[CallRecord],
    started: Instant,
    max_output_tokens: u64,
) -> ToolExecutionResult {
    let mut output = result.output.clone();
    if result.ok
        && let Some(value) = &result.value
    {
        output.push(OutputItem::Text {
            text: match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            },
            console: false,
        });
    }
    let mut items = format_output(&output);
    if !result.ok
        && let Some(error) = &result.error
    {
        items.push(text_block(format!(
            "Script error:\n{}",
            format_error(error, calls)
        )));
    }
    let truncated = truncate_output(join_adjacent_text(&items), max_output_tokens);
    let content = join_adjacent_text(&save_images(truncated.items));
    let wall = (Instant::now() - started).as_secs_f64();
    let header = format!(
        "{}\nWall time {wall:.1} seconds\nOutput:\n",
        if result.ok {
            "Script completed"
        } else {
            "Script failed"
        }
    );
    let mut details = json!({"calls": calls.iter().map(CallRecord::json).collect::<Vec<_>>()});
    if let Some(path) = truncated.full_output_path
        && let Some(object) = details.as_object_mut()
    {
        object.insert("fullOutputPath".to_owned(), json!(path));
    }
    let mut content_blocks = vec![text_block(header)];
    content_blocks.extend(content);
    ToolExecutionResult {
        content: Some(content_blocks),
        is_error: Some(!result.ok),
        details: Some(details),
        ..ToolExecutionResult::default()
    }
}

/// The result of a script that never started: the header, then why.
fn failed_result(wall: f64, error: &str, note: &str, calls: &[CallRecord]) -> ToolExecutionResult {
    ToolExecutionResult {
        content: Some(vec![
            text_block(format!(
                "Script failed\nWall time {wall:.1} seconds\nOutput:\n"
            )),
            text_block(format!("Script error:\n{error}\n\n{note}")),
        ]),
        is_error: Some(true),
        details: Some(json!({"calls": calls.iter().map(CallRecord::json).collect::<Vec<_>>()})),
        ..ToolExecutionResult::default()
    }
}

fn format_error(error: &pi_codemode::ScriptError, calls: &[CallRecord]) -> String {
    let head = match error.kind {
        ErrorKind::Script => error.stack.clone().unwrap_or_else(|| match &error.name {
            Some(name) => format!("{name}: {}", error.message),
            None => error.message.clone(),
        }),
        ErrorKind::Timeout => format!("Script timed out: {}", error.message),
        ErrorKind::Aborted => format!("Script aborted: {}", error.message),
        ErrorKind::Sandbox => format!("Script sandbox failed: {}", error.message),
    };
    let summary = if calls.is_empty() {
        "No tool calls were made.".to_owned()
    } else {
        format!(
            "Tool calls made before the failure (they are not undone): {}",
            calls
                .iter()
                .map(|call| format!("{} ({})", call.name, call.status.name()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!("{head}\n\n{summary}")
}

/// Lay the script's output out so the model can tell items apart: with more than one text item
/// each starts with a `==> text N/M <==` line, and `console` lines follow the rest in one block.
fn format_output(output: &[OutputItem]) -> Vec<UserBlock> {
    let total = output
        .iter()
        .filter(|item| matches!(item, OutputItem::Text { console: false, .. }))
        .count();
    let mut items: Vec<UserBlock> = Vec::new();
    let mut console: Vec<&str> = Vec::new();
    let mut index = 0usize;
    for item in output {
        match item {
            OutputItem::Image { data, mime_type } => {
                items.push(UserBlock::Image(ImageContent {
                    kind: ImageType,
                    data: data.clone(),
                    mime_type: mime_type.clone(),
                }));
            }
            OutputItem::Text {
                text,
                console: true,
            } => console.push(text),
            OutputItem::Text {
                text,
                console: false,
            } => {
                index += 1;
                items.push(text_block(if total > 1 {
                    format!("==> text {index}/{total} <==\n{text}")
                } else {
                    text.clone()
                }));
            }
        }
    }
    if !console.is_empty() {
        items.push(text_block(format!(
            "<console_output>\n{}\n</console_output>",
            console.join("\n")
        )));
    }
    items
}

/// Join adjacent text items into one, each part starting on its own line; images break a group.
fn join_adjacent_text(items: &[UserBlock]) -> Vec<UserBlock> {
    let mut joined: Vec<UserBlock> = Vec::new();
    let mut parts = String::new();
    let mut last_part = String::new();
    let flush = |parts: &mut String, joined: &mut Vec<UserBlock>| {
        if !parts.is_empty() {
            joined.push(text_block(std::mem::take(parts)));
        }
    };
    for item in items {
        let UserBlock::Text(text) = item else {
            flush(&mut parts, &mut joined);
            last_part.clear();
            joined.push(item.clone());
            continue;
        };
        if !last_part.is_empty() && !last_part.ends_with('\n') {
            parts.push('\n');
            last_part = "\n".to_owned();
        }
        parts.push_str(&text.text);
        if !text.text.is_empty() {
            last_part = text.text.clone();
        }
    }
    flush(&mut parts, &mut joined);
    joined
}

struct Truncated {
    items: Vec<UserBlock>,
    full_output_path: Option<String>,
}

/// Past the token budget the text becomes one item holding its start and end, and the whole text
/// goes to a temp file, as bash's truncated output does.
fn truncate_output(items: Vec<UserBlock>, max_tokens: u64) -> Truncated {
    let texts: Vec<&str> = items
        .iter()
        .filter_map(|item| match item {
            UserBlock::Text(text) => Some(text.text.as_str()),
            UserBlock::Image(_) => None,
        })
        .collect();
    let combined = texts.join("\n");
    let budget = max_tokens as usize * CHARS_PER_TOKEN;
    if texts.is_empty() || combined.chars().count() <= budget {
        return Truncated {
            items,
            full_output_path: None,
        };
    }
    let characters: Vec<char> = combined.chars().collect();
    let head_chars = budget / 2;
    let tail_chars = budget - head_chars;
    let head: String = characters.iter().take(head_chars).collect();
    let tail: String = characters
        .iter()
        .skip(characters.len().saturating_sub(tail_chars))
        .collect();
    let removed = characters.len().saturating_sub(head_chars + tail_chars);
    let mut text = format!(
        "Warning: truncated output (original token count: {})\nTotal output lines: {}\n\n{head}…{} tokens truncated…{tail}",
        combined.chars().count().div_ceil(CHARS_PER_TOKEN),
        combined.split('\n').count(),
        removed.div_ceil(CHARS_PER_TOKEN)
    );
    let spilled = spill(&combined);
    let (note, full_output_path) = match spilled {
        Ok(path) => (
            format!("\n\n[Full output: {path} (read with offset/limit)]"),
            Some(path),
        ),
        Err(error) => (
            format!("\n\n[Could not save the full output: {error}]"),
            None,
        ),
    };
    text.push_str(&note);
    let mut truncated = vec![text_block(text)];
    truncated.extend(
        items
            .into_iter()
            .filter(|item| matches!(item, UserBlock::Image(_))),
    );
    Truncated {
        items: truncated,
        full_output_path,
    }
}

fn spill(text: &str) -> std::result::Result<String, String> {
    let path = output_files::new_path("pi-codemode", ".txt");
    let mut file = output_files::create(&path).map_err(|error| error.to_string())?;
    file.write_all(text.as_bytes())
        .map_err(|error| error.to_string())?;
    Ok(path)
}

/// File extensions of the image types `image()` accepts.
fn image_extension(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "image/png" => Some(".png"),
        "image/jpeg" => Some(".jpg"),
        "image/gif" => Some(".gif"),
        "image/webp" => Some(".webp"),
        _ => None,
    }
}

fn base64_decode(data: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::with_capacity(data.len() * 3 / 4);
    let mut chunk = 0u32;
    let mut bits = 0u32;
    for byte in data.bytes() {
        let value = match byte {
            b'=' => break,
            other => TABLE.iter().position(|candidate| *candidate == other)? as u32,
        };
        chunk = (chunk << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((chunk >> bits) as u8);
        }
    }
    Some(bytes)
}

/// Save each image to a temp file and put a text item with its path before it: the model sees the
/// image but has no other way to reach its bytes.
fn save_images(items: Vec<UserBlock>) -> Vec<UserBlock> {
    let mut saved: Vec<(String, UserBlock)> = Vec::new();
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    let mut out = Vec::new();
    for item in items {
        let UserBlock::Image(image) = &item else {
            out.push(item);
            continue;
        };
        let label = match labels.get(&image.data) {
            Some(label) => label.clone(),
            None => {
                let bytes = base64_decode(&image.data).unwrap_or_default();
                let kind = format!("{}, {}", image.mime_type, format_size(bytes.len()));
                let label = match image_extension(&image.mime_type) {
                    Some(extension) => {
                        let path = output_files::new_path("pi-codemode", extension);
                        match output_files::create(&path)
                            .and_then(|mut file| file.write_all(&bytes))
                        {
                            Ok(()) => format!("[Image saved to {path} ({kind})]"),
                            Err(error) => format!("[Image ({kind}) could not be saved: {error}]"),
                        }
                    }
                    None => format!(
                        "[Image ({kind}) could not be saved: no file extension for image type {}]",
                        image.mime_type
                    ),
                };
                labels.insert(image.data.clone(), label.clone());
                label
            }
        };
        saved.push((label, item));
    }
    for (label, item) in saved {
        out.push(text_block(label));
        out.push(item);
    }
    out
}

enum Refusal {
    Aborted,
    Timeout(u64),
    Busy,
}

impl Refusal {
    fn message(&self, workers: usize) -> String {
        match self {
            Refusal::Aborted => "Script aborted: Execution aborted".to_owned(),
            Refusal::Timeout(ms) => format!(
                "Script timed out: Execution timed out after {ms} ms while waiting for a free codemode worker"
            ),
            Refusal::Busy => format!(
                "No codemode worker became free within {} seconds. All {workers} workers, shared by every session in this process, are held by other scripts. Retry after they finish.",
                WORKER_WAIT_MS / 1000
            ),
        }
    }
}

/// `PI_CODEMODE_MAX_WORKERS` if it is a positive integer, else 2.
fn max_workers() -> usize {
    std::env::var("PI_CODEMODE_MAX_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_WORKERS)
}

static WORKERS: Mutex<usize> = Mutex::new(0);
static WORKERS_FREE: Condvar = Condvar::new();

/// One script's claim on a worker slot, released when the script ends however it ends.
struct Workers;

impl Workers {
    /// Waits in arrival order while every slot is taken, and gives up at the deadline.
    fn acquire(deadline: Option<u64>, cancel: &CancelToken) -> std::result::Result<Self, Refusal> {
        let wait = deadline.map_or(WORKER_WAIT_MS, |ms| ms.min(WORKER_WAIT_MS));
        let started = Instant::now();
        let mut active = WORKERS.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if *active < max_workers() {
                *active += 1;
                return Ok(Self);
            }
            if cancel.is_cancelled() {
                return Err(Refusal::Aborted);
            }
            let elapsed = started.elapsed().as_millis() as u64;
            if elapsed >= wait {
                return Err(if deadline.is_some_and(|ms| ms <= WORKER_WAIT_MS) {
                    Refusal::Timeout(deadline.unwrap_or_default())
                } else {
                    Refusal::Busy
                });
            }
            let slice = Duration::from_millis(50).min(Duration::from_millis(wait - elapsed));
            active = WORKERS_FREE
                .wait_timeout(active, slice)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        let mut active = WORKERS.lock().unwrap_or_else(|error| error.into_inner());
        *active = active.saturating_sub(1);
        WORKERS_FREE.notify_one();
    }
}

/// `codemode.mode`: `on` keeps direct tools declared and says how a script calls them, `only`
/// lists every callable tool in the codemode description and declares no direct tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    On,
    Only,
}

impl Mode {
    pub fn parse(value: &str) -> Mode {
        match value {
            "only" => Mode::Only,
            _ => Mode::On,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::On => "on",
            Mode::Only => "only",
        }
    }
}

/// `codemode.inlineBudget`'s default, for a harness with no settings of its own.
pub fn default_inline_budget() -> f64 {
    DEFAULT_INLINE_BUDGET
}

/// One tool as the loadout sees it.
struct Entry {
    name: String,
    description: String,
    parameters: Value,
    callable: bool,
    direct: bool,
    deferred: bool,
}

const DESCRIPTION_INTRO_END: &str = "\n\n";

fn output_schema(tool: &pi_durable::ai::types::Tool) -> Value {
    let _ = tool;
    json!({"type": "string"})
}

fn describe_output(_schema: &Value) -> &'static str {
    "a string"
}

/// A tool's section in the codemode description: `### `id`` then its sample.
fn render_tool_section(entry: &Entry) -> String {
    let id = declarations::to_identifier(&entry.name);
    let heading = if id == entry.name {
        format!("### `{id}`")
    } else {
        format!("### `{id}` (`{}`)", entry.name)
    };
    let sample = declarations::render_sample(
        &entry.name,
        Some(&entry.description),
        Some(&entry.parameters),
        Some(&output_schema(&pi_durable::ai::types::Tool {
            name: entry.name.clone(),
            description: entry.description.clone(),
            parameters: entry.parameters.clone(),
            constrained_sampling: None,
        })),
    );
    format!("{heading}\n{}", sample.trim())
}

/// Pick the sections that fit the budget, cheapest first: a tool that does not fit is left out.
fn select_catalog(entries: &[&Entry], budget: Option<f64>) -> Vec<String> {
    let Some(budget) = budget else {
        return entries.iter().map(|entry| entry.name.clone()).collect();
    };
    let mut costs: Vec<(usize, f64)> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let cost = (render_tool_section(entry).len() as f64 / CHARS_PER_TOKEN as f64).ceil();
            (index, cost)
        })
        .collect();
    costs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut chosen = Vec::new();
    let mut remaining = budget;
    for (index, cost) in costs {
        if cost > remaining {
            continue;
        }
        remaining -= cost;
        chosen.push(index);
    }
    chosen.sort_unstable();
    chosen
        .into_iter()
        .map(|index| entries[index].name.clone())
        .collect()
}

/// The model-facing description: the intro, the globals, and one section per listed tool.
fn create_description(entries: &[Entry], budget: Option<f64>) -> String {
    let listed: Vec<&Entry> = entries.iter().filter(|entry| !entry.deferred).collect();
    let sections = [DESCRIPTION_INTRO, GLOBALS];
    if listed.is_empty() {
        return sections.join(DESCRIPTION_INTRO_END);
    }
    let shown = select_catalog(&listed, budget);
    let mut body = vec!["Nested tools:".to_owned()];
    for entry in listed {
        if shown.contains(&entry.name) {
            body.push(render_tool_section(entry));
        }
    }
    let mut all: Vec<String> = sections
        .iter()
        .map(|section| (*section).to_owned())
        .collect();
    all.push(body.join("\n\n"));
    all.join(DESCRIPTION_INTRO_END)
}

/// `prepareCodemodeLoadout`: what the model is told about each tool, and which declared tools the
/// request leaves out while keeping them callable.
pub fn apply_loadout(tools: &mut [pi_durable::harness::types::ToolRegistration], mode: Mode) {
    use pi_durable::harness::types::ToolExposure;
    let Some(codemode_index) = tools.iter().position(|tool| tool.tool.name == TOOL_NAME) else {
        return;
    };
    let entries: Vec<Entry> = tools
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != codemode_index)
        .map(|(_, tool)| Entry {
            name: tool.tool.name.clone(),
            description: tool.tool.description.clone(),
            parameters: tool.tool.parameters.clone(),
            callable: tool.exposure.callable(true),
            direct: tool.exposure == ToolExposure::Direct,
            deferred: tool.exposure == ToolExposure::Deferred,
        })
        .collect();
    if mode == Mode::On {
        for entry in entries
            .iter()
            .filter(|entry| entry.direct && entry.callable)
        {
            if let Some(tool) = tools.iter_mut().find(|tool| tool.tool.name == entry.name) {
                tool.tool.description = format!(
                    "{}\n\nCodemode: `tools.{}(args)` resolves to {}.",
                    tool.tool.description.trim(),
                    declarations::to_identifier(&entry.name),
                    describe_output(&output_schema(&tool.tool))
                );
            }
        }
    }
    let listed: Vec<Entry> = entries
        .iter()
        .filter(|entry| entry.callable && (mode == Mode::Only || !entry.direct))
        .map(|entry| Entry {
            name: entry.name.clone(),
            description: entry.description.clone(),
            parameters: entry.parameters.clone(),
            callable: entry.callable,
            direct: entry.direct,
            deferred: entry.deferred,
        })
        .collect();
    let description = create_description(&listed, Some(inline_budget()));
    tools[codemode_index].tool.description = description;
    if mode == Mode::Only {
        for entry in entries
            .iter()
            .filter(|entry| entry.direct && entry.callable)
        {
            if let Some(tool) = tools.iter_mut().find(|tool| tool.tool.name == entry.name) {
                tool.hidden = true;
            }
        }
    }
}

/// The token budget for the description's tool sections, from `codemode.inlineBudget`.
static INLINE_BUDGET: Mutex<Option<f64>> = Mutex::new(None);

fn inline_budget() -> f64 {
    INLINE_BUDGET
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .unwrap_or(DEFAULT_INLINE_BUDGET)
}

/// Sets the description's token budget for this process, as `codemode.inlineBudget` says.
pub fn set_inline_budget(budget: f64) {
    *INLINE_BUDGET
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(budget);
}

/// The tools the model's request declares: with `only`, codemode alone, since direct tools are
/// hidden declarations. The prompt's tool list is built from this, as upstream's is.
pub fn declared_tools(tools: &[String], mode: Mode) -> Vec<String> {
    if mode == Mode::Only && tools.iter().any(|tool| tool == TOOL_NAME) {
        return vec![TOOL_NAME.to_owned()];
    }
    tools.to_vec()
}
