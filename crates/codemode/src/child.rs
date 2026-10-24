//! The sandbox child: one script per process, spoken to over piped stdio
//! (`packages/codemode/src/runtime/vm.ts`, which the worker in `host.ts` drives).
//!
//! QuickJS runs synchronously, so a script can block its thread; the parent enforces the deadline
//! by killing this process rather than asking the VM to stop.

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::rc::Rc;

use rquickjs::context::EvalOptions;
use rquickjs::function::MutFn;
use rquickjs::{Context, Ctx, Exception, Function, Object, Runtime, Value};

use crate::protocol::{CallTarget, OutputItem, VmStartData, WorkerToHost};

const PRELUDE: &str = include_str!("prelude.js");

/// The hidden `--codemode-vm` mode: read one start message, run its script, report, exit.
pub fn run() -> i32 {
    match child() {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("pi-rs: codemode: {message}");
            1
        }
    }
}

fn child() -> Result<(), String> {
    let mut input = std::io::stdin().lock();
    let mut first = String::new();
    input.read_line(&mut first).map_err(|e| e.to_string())?;
    let start: VmStartData =
        serde_json::from_str(&first).map_err(|e| format!("bad start message: {e}"))?;
    let runtime = Runtime::new().map_err(|e| describe(&e))?;
    if let Some(limit) = start.memory_limit_bytes {
        runtime.set_memory_limit(limit);
    }
    let context = Context::full(&runtime).map_err(|e| describe(&e))?;
    let mut out = std::io::stdout().lock();
    context.with(|ctx| run_script(&ctx, &start, &mut out))
}

fn options(filename: &str) -> EvalOptions {
    let mut options = EvalOptions::default();
    options.global = true;
    options.filename = Some(filename.to_owned());
    options
}

fn run_script(ctx: &Ctx<'_>, start: &VmStartData, out: &mut impl Write) -> Result<(), String> {
    let queue: Rc<RefCell<Vec<WorkerToHost>>> = Rc::new(RefCell::new(Vec::new()));
    let bridge = {
        let queue = Rc::clone(&queue);
        Function::new(
            ctx.clone(),
            MutFn::from(
                move |kind: String, a: Value<'_>, b: Option<String>, c: Option<String>| {
                    let message = match kind.as_str() {
                        "call" | "global" => WorkerToHost::Call {
                            id: a.as_number().unwrap_or(0.0) as u64,
                            target: if kind == "call" {
                                CallTarget::Tool
                            } else {
                                CallTarget::Global
                            },
                            name: b.unwrap_or_default(),
                            args: c,
                        },
                        "output" => WorkerToHost::Output {
                            item: match a.as_string().and_then(|s| s.to_string().ok()) {
                                Some(kind) if kind == "image" => OutputItem::Image {
                                    data: b.unwrap_or_default(),
                                    mime_type: c.unwrap_or_default(),
                                },
                                Some(kind) => {
                                    let console = kind == "console";
                                    OutputItem::Text {
                                        text: b.unwrap_or_default(),
                                        console,
                                    }
                                }
                                None => return,
                            },
                        },
                        "done" => {
                            let ok = a.as_bool().unwrap_or(false);
                            let (value, error) = if ok { (b, None) } else { (None, b) };
                            WorkerToHost::Done {
                                ok,
                                value,
                                writes: c,
                                error,
                            }
                        }
                        _ => return,
                    };
                    queue.borrow_mut().push(message);
                },
            ),
        )
        .map_err(|e| describe(&e))?
    };

    let factory: Function = ctx
        .eval_with_options(PRELUDE, options("codemode-prelude.js"))
        .map_err(|e| describe(&e))?;
    let api: Object = factory.call((bridge,)).map_err(|e| describe(&e))?;

    let source = format!("(async (tools, console) => {{{}\n}})", start.code);
    let script: Function = match ctx.eval_with_options(source, options("codemode.js")) {
        Ok(script) => script,
        Err(error) => {
            let described = exception_json(ctx, &error);
            return write_messages(
                out,
                vec![WorkerToHost::Done {
                    ok: false,
                    value: None,
                    writes: None,
                    error: Some(described),
                }],
            );
        }
    };
    let run: Function = api.get("run").map_err(|e| describe(&e))?;
    run.call::<_, ()>((script,)).map_err(|e| describe(&e))?;
    drain(ctx);
    let queued: Vec<WorkerToHost> = queue.borrow_mut().drain(..).collect();
    write_messages(out, queued)
}

fn drain(ctx: &Ctx<'_>) {
    while ctx.execute_pending_job() {}
}

fn write_messages(out: &mut impl Write, messages: Vec<WorkerToHost>) -> Result<(), String> {
    for message in messages {
        out.write_all(message.to_line().as_bytes())
            .map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())
}

/// A JSON object whose own properties the script's `Error` gets, as the host's runtime does.
fn exception_json(ctx: &Ctx<'_>, error: &rquickjs::Error) -> String {
    match error {
        rquickjs::Error::Exception => {
            let caught = ctx.catch();
            match caught.as_exception() {
                Some(exception) => describe_exception(exception),
                None => {
                    serde_json::json!({ "message": caught.get::<String>().unwrap_or_default() })
                        .to_string()
                }
            }
        }
        other => serde_json::json!({ "message": describe(other) }).to_string(),
    }
}

/// `describeException`: "Name: message" followed by the frames, as V8's stacks read.
fn describe_exception(exception: &Exception<'_>) -> String {
    let name: String = exception
        .get("name")
        .ok()
        .flatten()
        .unwrap_or_else(|| "Error".to_owned());
    let message = exception.message().unwrap_or_default();
    let head = if message.is_empty() {
        name.clone()
    } else {
        format!("{name}: {message}")
    };
    let stack = match exception.stack() {
        Some(frames) if !frames.trim().is_empty() => format!("{head}\n{frames}"),
        _ => head,
    };
    serde_json::json!({ "name": name, "message": message, "stack": stack }).to_string()
}

fn describe(error: &rquickjs::Error) -> String {
    match error {
        rquickjs::Error::Exception => "JavaScript exception".to_owned(),
        other => format!("{other:?}"),
    }
}
