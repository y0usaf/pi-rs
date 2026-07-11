//! Differential Pi/pi-rs terminal-cell harness.

use std::io::{Read as _, Write as _};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::{fs, path::PathBuf, process::ExitCode};

use pi_rs_host::{Host, HostConfig};
use pi_rs_tui::ui_harness::{FrameRecorder, FrameSnapshot, first_diff};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RawFrame {
    name: String,
    columns: u16,
    rows: u16,
    ansi: String,
}

#[derive(Debug, Deserialize)]
struct RawSequence {
    frames: Vec<RawFrame>,
}

#[derive(Debug, thiserror::Error)]
enum HarnessError {
    #[error("I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("pi-rs host failed: {0}")]
    Host(#[from] pi_rs_host::HostError),
    #[error("pi-rs parity command returned no result")]
    MissingResult,
    #[error("invalid arguments: {0}")]
    Arguments(String),
}

fn read(path: &PathBuf) -> Result<String, HarnessError> {
    fs::read_to_string(path).map_err(|source| HarnessError::Io {
        path: path.clone(),
        source,
    })
}

/// Render one scripted stub response (tests/ui-parity scenario `stub`
/// vocabulary, shared with tests/anthropic-parity/cases.json) to raw
/// HTTP bytes plus its hang flag.
fn scripted_response(
    response: &serde_json::Value,
    shared_sse: &serde_json::Value,
) -> (String, bool) {
    let status = response["status"].as_u64().unwrap_or(200);
    let hang = response["hang"].as_bool().unwrap_or(false);
    let events = response
        .get("sse")
        .and_then(serde_json::Value::as_str)
        .map(|name| shared_sse[name].clone())
        .or_else(|| response.get("events").cloned());
    let (body, content_type) = if let Some(events) = events {
        let body: String = events
            .as_array()
            .map(|events| {
                events
                    .iter()
                    .map(|event| {
                        format!(
                            "event: {}\ndata: {}\n\n",
                            event["event"].as_str().unwrap_or_default(),
                            event["data"]
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        (body, "text/event-stream")
    } else if let Some(json_body) = response.get("json") {
        (json_body.to_string(), "application/json")
    } else {
        (
            response
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            "text/plain",
        )
    };
    let head = format!("HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\n");
    if hang {
        // No content-length: the body runs until connection close, which
        // never comes — the client must abort.
        (format!("{head}\r\n{body}"), true)
    } else {
        (
            format!(
                "{head}content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            ),
            false,
        )
    }
}

/// Read one HTTP request (headers plus content-length body), blocking.
fn read_stub_request(sock: &mut std::net::TcpStream) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = match sock.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
            let content_length: usize = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            while buf.len() - (pos + 4) < content_length {
                match sock.read(&mut tmp) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                }
            }
            return;
        }
    }
}

/// Serve the scenario's scripted provider responses (one connection per
/// request, in order, last response repeated), exactly like the pi
/// driver's node stub. Hanging sockets stay open until the client aborts.
fn spawn_stub(stub: &serde_json::Value) -> Result<String, HarnessError> {
    let shared_sse = stub.get("sse").cloned().unwrap_or(serde_json::Value::Null);
    let responses: Vec<(String, bool)> = stub["responses"]
        .as_array()
        .map(|responses| {
            responses
                .iter()
                .map(|response| scripted_response(response, &shared_sse))
                .collect()
        })
        .unwrap_or_default();
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").map_err(|source| HarnessError::Io {
            path: PathBuf::from("<stub listener>"),
            source,
        })?;
    let addr = listener.local_addr().map_err(|source| HarnessError::Io {
        path: PathBuf::from("<stub listener>"),
        source,
    })?;
    std::thread::spawn(move || {
        for (index, conn) in listener.incoming().enumerate() {
            let Ok(mut sock) = conn else { break };
            read_stub_request(&mut sock);
            let Some((response, hang)) = responses.get(index).or_else(|| responses.last()) else {
                break;
            };
            let _ = sock.write_all(response.as_bytes());
            if *hang {
                // Park the connection on its own thread; the read returns
                // when the client aborts and drops the socket.
                std::thread::spawn(move || {
                    let mut sink = [0u8; 64];
                    let _ = sock.read(&mut sink);
                });
            } else {
                let _ = sock.shutdown(std::net::Shutdown::Both);
            }
        }
    });
    Ok(format!("http://{addr}"))
}

fn raw_snapshots(sequence: RawSequence) -> Vec<FrameSnapshot> {
    let Some(first) = sequence.frames.first() else {
        return Vec::new();
    };
    let mut recorder = FrameRecorder::new(first.columns, first.rows);
    sequence
        .frames
        .into_iter()
        .map(|frame| {
            recorder.resize(frame.columns, frame.rows);
            recorder.process(frame.ansi.as_bytes());
            recorder.snapshot(frame.name)
        })
        .collect()
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("ui-diff: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, HarnessError> {
    let mut args = std::env::args_os().skip(1);
    let scenario_path = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "tests/ui-parity/basic-turn.json".into()),
    );
    let oracle_path = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "tests/ui-parity/basic-turn.pi.json".into()),
    );
    if let Some(raw_path) = args.next() {
        if args.next().is_some() {
            return Err(HarnessError::Arguments(
                "expected [scenario] [oracle] [Pi raw capture]".to_owned(),
            ));
        }
        let raw_path = PathBuf::from(raw_path);
        let raw: RawSequence =
            serde_json::from_str(&read(&raw_path)?).map_err(|source| HarnessError::Json {
                path: raw_path,
                source,
            })?;
        let encoded = serde_json::to_string_pretty(&raw_snapshots(raw)).map_err(|source| {
            HarnessError::Json {
                path: oracle_path.clone(),
                source,
            }
        })?;
        fs::write(&oracle_path, format!("{encoded}\n")).map_err(|source| HarnessError::Io {
            path: oracle_path.clone(),
            source,
        })?;
        println!("updated Pi oracle {}", oracle_path.display());
        return Ok(ExitCode::SUCCESS);
    }

    let mut scenario: serde_json::Value =
        serde_json::from_str(&read(&scenario_path)?).map_err(|source| HarnessError::Json {
            path: scenario_path.clone(),
            source,
        })?;
    let expected: Vec<FrameSnapshot> =
        serde_json::from_str(&read(&oracle_path)?).map_err(|source| HarnessError::Json {
            path: oracle_path.clone(),
            source,
        })?;
    // Scenario files land in a fresh temp cwd, mirroring the Pi driver's
    // mkdtemp; renderers only surface the relative paths from the args.
    // Session-UI scenarios (PLAN 6.3) instead pin `fixedCwd` — an absolute
    // recreated-per-run directory — because /session and the cwd prompt
    // surface absolute paths that must match the recorded oracle.
    let mut scenario_dir = None;
    if let Some(files) = scenario.get("files").and_then(|v| v.as_object()).cloned() {
        let root: PathBuf =
            if let Some(fixed) = scenario.get("fixedCwd").and_then(serde_json::Value::as_str) {
                let fixed = PathBuf::from(fixed);
                let io_error = |source| HarnessError::Io {
                    path: fixed.clone(),
                    source,
                };
                if fixed.exists() {
                    fs::remove_dir_all(&fixed).map_err(io_error)?;
                }
                fs::create_dir_all(&fixed).map_err(io_error)?;
                fixed
            } else {
                let dir = tempfile::tempdir().map_err(|source| HarnessError::Io {
                    path: scenario_path.clone(),
                    source,
                })?;
                let path = dir.path().to_path_buf();
                scenario_dir = Some(dir);
                path
            };
        for (name, contents) in &files {
            let path = root.join(name);
            let io_error = |source| HarnessError::Io {
                path: path.clone(),
                source,
            };
            if name.ends_with('/') {
                fs::create_dir_all(&path).map_err(io_error)?;
                continue;
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(io_error)?;
            }
            fs::write(&path, contents.as_str().unwrap_or_default()).map_err(io_error)?;
        }
        #[cfg(unix)]
        if let Some(executables) = scenario
            .get("executables")
            .and_then(|value| value.as_array())
        {
            for name in executables.iter().filter_map(serde_json::Value::as_str) {
                let path = root.join(name);
                let io_error = |source| HarnessError::Io {
                    path: path.clone(),
                    source,
                };
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).map_err(io_error)?;
            }
        }
        let cwd = root.to_string_lossy().into_owned();
        // Resume scenarios (PLAN 6.2): the session fixture materializes
        // with the file tree; the product opens it by absolute path (the
        // pi driver joins its temp cwd the same way). `sessionDir` and
        // `cwdSubdir` join the same root (PLAN 6.3).
        for key in ["sessionFile", "sessionDir"] {
            if let Some(name) = scenario
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
            {
                scenario[key] =
                    serde_json::Value::String(root.join(name).to_string_lossy().into_owned());
            }
        }
        scenario["cwd"] = serde_json::Value::String(cwd);
    }
    let _scenario_dir = scenario_dir;
    // Deterministic process environment (e.g. PATH="" so the delete flow's
    // `trash` probe fails identically on both sides).
    if let Some(env) = scenario.get("env").and_then(|v| v.as_object()) {
        for (key, value) in env {
            // SAFETY: single-threaded at this point; the Lua host starts below.
            unsafe { std::env::set_var(key, value.as_str().unwrap_or_default()) };
        }
    }
    // The provider scenario's footer shows the scenario cwd as "~": both
    // drivers point HOME at the temp cwd so real (per-run) paths never
    // reach the frames.
    if scenario
        .get("homeFromCwd")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        scenario["home"] = scenario["cwd"].clone();
    }
    // Real-provider scenarios (PLAN 4.2): serve the scripted SSE stub and
    // point the scenario model at it, like the pi driver's node stub.
    if let Some(stub) = scenario.get("stub").cloned() {
        let base_url = spawn_stub(&stub)?;
        scenario["model"]["baseUrl"] = serde_json::Value::String(base_url);
    }
    // Deterministic auth/registry state: the product wiring reads the
    // coding-agent dir at Host::new; pin it to an empty temp dir so
    // ambient credentials on the developer machine cannot change frames
    // (the pi driver pins HOME the same way).
    let agent_dir = tempfile::tempdir().map_err(|source| HarnessError::Io {
        path: scenario_path.clone(),
        source,
    })?;
    // SAFETY: single-threaded at this point; the Lua host starts below.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", agent_dir.path()) };
    let command = scenario
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("interactive-parity-sequence")
        .to_owned();
    // Deterministic environment, matching the Pi drivers' setCapabilities
    // pin: no images, truecolor, no OSC 8 hyperlinks.
    pi_rs_tui::terminal_image::set_capabilities(pi_rs_tui::terminal_image::TerminalCapabilities {
        images: None,
        true_color: true,
        hyperlinks: false,
    });
    // Tools execute against the host cwd (`pi.cwd()`); scenarios with a
    // temp file tree run there, like the pi driver's process.chdir.
    let host = Host::new(HostConfig {
        cwd: scenario
            .get("cwd")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        ..HostConfig::default()
    })?;
    let report = host.load_embedded(&[
        pi_rs_agent::PACK,
        pi_rs_app::builtins::TOOLS_PACK,
        pi_rs_app::builtins::INTERACTIVE_PACK,
    ]);
    if let Some(error) = report.errors.first() {
        return Err(HarnessError::Arguments(format!(
            "builtin pack failed to load: {}",
            error.error
        )));
    }
    let scenario = serde_json::to_string(&scenario).map_err(|source| HarnessError::Json {
        path: scenario_path.clone(),
        source,
    })?;
    let result = host
        .call_command(&command, &scenario)?
        .ok_or(HarnessError::MissingResult)?;
    let raw: RawSequence = serde_json::from_value(result).map_err(|source| HarnessError::Json {
        path: scenario_path,
        source,
    })?;
    let actual = raw_snapshots(raw);
    // Debug aid: UI_DIFF_DUMP=<checkpoint> prints pi-rs's rendered rows.
    if let Some(dump) = std::env::var_os("UI_DIFF_DUMP") {
        let wanted = dump.to_string_lossy();
        for frame in &actual {
            if frame.name == wanted {
                for row in 0..frame.rows as usize {
                    let line: String = frame.cells
                        [row * frame.columns as usize..(row + 1) * frame.columns as usize]
                        .iter()
                        .map(|cell| cell.text.as_str())
                        .collect();
                    println!("{row:2}|{}", line.trim_end());
                }
                println!("cursor {} {}", frame.cursor_row, frame.cursor_column);
            }
        }
    }
    if let Some(diff) = first_diff(&expected, &actual) {
        eprintln!(
            "UI mismatch at checkpoint {:?}:\n{}",
            diff.checkpoint, diff.message
        );
        return Ok(ExitCode::FAILURE);
    }
    println!("Pi/pi-rs UI cells match at {} checkpoints", actual.len());
    Ok(ExitCode::SUCCESS)
}
