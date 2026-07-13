//! Reproducible release-mode performance baseline harness.

use std::fs;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use pi_rs_host::{Host, HostConfig};
use pi_rs_tui::terminal::TerminalState;
use pi_rs_tui::tui::Tui;
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "pi-rs-performance-v1";
const LUA_BENCH: &str = r#"
local pi = ...
pi.on("bench-noop", function(_) return nil end)
pi.on("bench-batch", function(event)
  return { actions = {
    { type = "status", value = event.value },
    { type = "invalidate", target = "editor" },
    { type = "effect", name = "local" }
  } }
end)
pi.on("bench-effect", function(_)
  pi.sleep(0)
  return { completed = true }
end)
"#;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema: String,
    fixture_revision: String,
    warmup_runs: usize,
    startup_samples: usize,
    idle_samples: usize,
    idle_settle_ms: u64,
    input_samples: usize,
    render_samples: usize,
    lua_samples: usize,
    effect_samples: usize,
}

#[derive(Debug, Serialize)]
struct Report {
    schema: &'static str,
    source_revision: String,
    fixture_revision: String,
    profile: &'static str,
    environment: Environment,
    conditions: Conditions,
    parameters: Parameters,
    metrics: Metrics,
}

#[derive(Debug, Serialize)]
struct Environment {
    os: &'static str,
    architecture: &'static str,
    cpu: String,
    logical_cpus: usize,
    total_memory_mib: Option<u64>,
}

#[derive(Debug, Serialize)]
struct Conditions {
    startup: &'static str,
    idle: String,
    input: &'static str,
    render: &'static str,
    lua: &'static str,
    effect: &'static str,
}

#[derive(Debug, Serialize)]
struct Parameters {
    warmup_runs: usize,
    startup_samples: usize,
    idle_samples: usize,
    input_samples: usize,
    render_samples: usize,
    lua_samples: usize,
    effect_samples: usize,
}

#[derive(Debug, Serialize)]
struct Metrics {
    startup_ms: Distribution,
    idle_rss_mib: Distribution,
    input_to_frame_us: Distribution,
    retained_render_unchanged_us: Distribution,
    retained_render_changed_us: Distribution,
    retained_render_changed_frames_per_second: Scalar,
    lua_dispatch_noop_us: Distribution,
    lua_dispatch_action_batch_us: Distribution,
    lua_snapshot_bytes: Scalar,
    effect_round_trip_us: Distribution,
}

#[derive(Debug, Serialize)]
struct Distribution {
    unit: &'static str,
    samples: usize,
    min: f64,
    p50: f64,
    p95: f64,
    max: f64,
}

#[derive(Debug, Serialize)]
struct Scalar {
    unit: &'static str,
    value: f64,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("release mode is required (run the harness with --release)")]
    DebugBuild,
    #[error("usage: performance-baseline --config FILE [--output FILE] | --self-test | --probe")]
    Usage,
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
    #[error("invalid benchmark config: {0}")]
    Config(String),
    #[error("probe failed: {0}")]
    Probe(String),
    #[error("host failed: {0}")]
    Host(#[from] pi_rs_host::HostError),
    #[error("TUI failed: {0}")]
    Tui(#[from] pi_rs_tui::tui::TuiError),
}

struct Probe {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Probe {
    fn spawn() -> Result<(Self, Duration), Error> {
        let executable = std::env::current_exe().map_err(|source| Error::Io {
            path: PathBuf::from("<current executable>"),
            source,
        })?;
        let started = Instant::now();
        let mut child = Command::new(executable)
            .arg("--probe")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| Error::Io {
                path: PathBuf::from("<performance probe>"),
                source,
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Probe("probe stdin unavailable".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Probe("probe stdout unavailable".to_owned()))?;
        let mut probe = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        };
        let line = probe.read_line()?;
        if line != "READY" {
            return Err(Error::Probe(format!("expected READY, got {line:?}")));
        }
        Ok((probe, started.elapsed()))
    }

    fn read_line(&mut self) -> Result<String, Error> {
        let mut line = String::new();
        let count = self
            .stdout
            .read_line(&mut line)
            .map_err(|source| Error::Io {
                path: PathBuf::from("<probe stdout>"),
                source,
            })?;
        if count == 0 {
            return Err(Error::Probe("probe exited before replying".to_owned()));
        }
        Ok(line.trim_end().to_owned())
    }

    fn input_round_trip(&mut self, sequence: usize) -> Result<Duration, Error> {
        let started = Instant::now();
        writeln!(self.stdin, "INPUT {sequence}").map_err(|source| Error::Io {
            path: PathBuf::from("<probe stdin>"),
            source,
        })?;
        self.stdin.flush().map_err(|source| Error::Io {
            path: PathBuf::from("<probe stdin>"),
            source,
        })?;
        let expected = format!("FRAME {sequence}");
        let actual = self.read_line()?;
        if actual != expected {
            return Err(Error::Probe(format!(
                "expected {expected:?}, got {actual:?}"
            )));
        }
        Ok(started.elapsed())
    }

    fn rss_mib(&self) -> Result<f64, Error> {
        let path = PathBuf::from(format!("/proc/{}/status", self.child.id()));
        let status = fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        let kib = status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| Error::Probe(format!("VmRSS missing from {}", path.display())))?;
        Ok(kib as f64 / 1024.0)
    }

    fn stop(mut self) -> Result<(), Error> {
        writeln!(self.stdin, "QUIT").map_err(|source| Error::Io {
            path: PathBuf::from("<probe stdin>"),
            source,
        })?;
        self.stdin.flush().map_err(|source| Error::Io {
            path: PathBuf::from("<probe stdin>"),
            source,
        })?;
        let status = self.child.wait().map_err(|source| Error::Io {
            path: PathBuf::from("<performance probe>"),
            source,
        })?;
        if !status.success() {
            return Err(Error::Probe(format!("probe exited with {status}")));
        }
        Ok(())
    }
}

fn probe_main() -> Result<(), Error> {
    let host = Host::new(HostConfig::default())?;
    host.load("<performance-probe>", LUA_BENCH)?;
    let mut tui = Tui::new(TerminalState::new(Some(100), Some(30)), true);
    tui.start();
    let _ = tui.render_if_requested(vec![
        "pi-rs performance probe".to_owned(),
        format!("input:0{}", pi_rs_tui::tui::CURSOR_MARKER),
    ])?;
    let _ = tui.take_output();
    println!("READY");
    std::io::stdout().flush().map_err(|source| Error::Io {
        path: PathBuf::from("<stdout>"),
        source,
    })?;

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(|source| Error::Io {
            path: PathBuf::from("<stdin>"),
            source,
        })?;
        if line == "QUIT" {
            break;
        }
        let sequence = line
            .strip_prefix("INPUT ")
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| Error::Probe(format!("invalid probe command {line:?}")))?;
        let _ = tui.feed_input(b"x");
        tui.request_render(false);
        let _ = tui.render_if_requested(vec![
            "pi-rs performance probe".to_owned(),
            format!("input:{sequence}{}", pi_rs_tui::tui::CURSOR_MARKER),
        ])?;
        let _ = tui.take_output();
        println!("FRAME {sequence}");
        std::io::stdout().flush().map_err(|source| Error::Io {
            path: PathBuf::from("<stdout>"),
            source,
        })?;
    }
    drop(host);
    Ok(())
}

fn validate_config(config: &Config) -> Result<(), Error> {
    if config.schema != SCHEMA {
        return Err(Error::Config(format!(
            "expected schema {SCHEMA:?}, got {:?}",
            config.schema
        )));
    }
    let counts = [
        config.startup_samples,
        config.idle_samples,
        config.input_samples,
        config.render_samples,
        config.lua_samples,
        config.effect_samples,
    ];
    if counts.contains(&0) {
        return Err(Error::Config("sample counts must be non-zero".to_owned()));
    }
    Ok(())
}

fn read_config(path: &Path) -> Result<Config, Error> {
    let bytes = fs::read(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let config = serde_json::from_slice(&bytes).map_err(|source| Error::Json {
        path: path.to_path_buf(),
        source,
    })?;
    validate_config(&config)?;
    Ok(config)
}

fn percentile_index(len: usize, percentile: usize) -> usize {
    ((len * percentile).div_ceil(100)).saturating_sub(1)
}

fn distribution(mut values: Vec<f64>, unit: &'static str) -> Result<Distribution, Error> {
    if values.is_empty()
        || values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(Error::Config(
            "metric samples must be finite and non-empty".to_owned(),
        ));
    }
    values.sort_by(f64::total_cmp);
    Ok(Distribution {
        unit,
        samples: values.len(),
        min: values[0],
        p50: values[percentile_index(values.len(), 50)],
        p95: values[percentile_index(values.len(), 95)],
        max: values[values.len() - 1],
    })
}

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn benchmark_render(samples: usize) -> Result<(Distribution, Distribution, Scalar), Error> {
    let mut tui = Tui::new(TerminalState::new(Some(120), Some(40)), false);
    let base: Vec<String> = (0..80)
        .map(|row| format!("retained row {row:02} {}", "x".repeat(80)))
        .collect();
    tui.start();
    let _ = tui.render_if_requested(base.clone())?;
    let _ = tui.take_output();

    let mut unchanged = Vec::with_capacity(samples);
    for _ in 0..samples {
        tui.request_render(false);
        let started = Instant::now();
        let _ = tui.render_if_requested(base.clone())?;
        let _ = tui.take_output();
        unchanged.push(micros(started.elapsed()));
    }

    let mut changed = Vec::with_capacity(samples);
    let throughput_started = Instant::now();
    for iteration in 0..samples {
        let mut frame = base.clone();
        frame[40] = format!("retained changed frame {iteration:08}");
        tui.request_render(false);
        let started = Instant::now();
        let _ = tui.render_if_requested(frame)?;
        let _ = tui.take_output();
        changed.push(micros(started.elapsed()));
    }
    let elapsed = throughput_started.elapsed().as_secs_f64();
    Ok((
        distribution(unchanged, "us")?,
        distribution(changed, "us")?,
        Scalar {
            unit: "frames/s",
            value: samples as f64 / elapsed,
        },
    ))
}

fn checked_emit(host: &Host, event: &str, payload: &serde_json::Value) -> Result<(), Error> {
    let outcomes = host.emit(event, payload)?;
    let first = outcomes
        .first()
        .ok_or_else(|| Error::Probe(format!("event {event:?} had no handler")))?;
    if let Err(message) = &first.result {
        return Err(Error::Probe(format!("event {event:?} failed: {message}")));
    }
    Ok(())
}

fn benchmark_lua(
    lua_samples: usize,
    effect_samples: usize,
) -> Result<(Distribution, Distribution, Scalar, Distribution), Error> {
    let host = Host::new(HostConfig::default())?;
    host.load("<performance-benchmark>", LUA_BENCH)?;
    let payload = serde_json::json!({
        "generation": 7,
        "value": "bounded snapshot",
        "selection": { "start": 4, "end": 9 },
        "items": [1, 2, 3, 4]
    });
    checked_emit(&host, "bench-noop", &payload)?;
    checked_emit(&host, "bench-batch", &payload)?;
    checked_emit(&host, "bench-effect", &payload)?;

    let mut noop = Vec::with_capacity(lua_samples);
    let mut batch = Vec::with_capacity(lua_samples);
    for _ in 0..lua_samples {
        let started = Instant::now();
        checked_emit(&host, "bench-noop", &payload)?;
        noop.push(micros(started.elapsed()));
    }
    for _ in 0..lua_samples {
        let started = Instant::now();
        checked_emit(&host, "bench-batch", &payload)?;
        batch.push(micros(started.elapsed()));
    }
    let mut effect = Vec::with_capacity(effect_samples);
    for _ in 0..effect_samples {
        let started = Instant::now();
        checked_emit(&host, "bench-effect", &payload)?;
        effect.push(micros(started.elapsed()));
    }
    let snapshot_bytes = serde_json::to_vec(&payload)
        .map_err(|error| Error::Config(format!("serialize benchmark snapshot: {error}")))?
        .len();
    Ok((
        distribution(noop, "us")?,
        distribution(batch, "us")?,
        Scalar {
            unit: "bytes/dispatch",
            value: snapshot_bytes as f64,
        },
        distribution(effect, "us")?,
    ))
}

fn source_revision() -> String {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|rest| rest.split_once(':'))
                    .map(|(_, value)| value.trim().to_owned())
            })
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn total_memory_mib() -> Option<u64> {
    fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
        .map(|kib| kib / 1024)
}

fn benchmark(config: Config) -> Result<Report, Error> {
    for _ in 0..config.warmup_runs {
        let (probe, _) = Probe::spawn()?;
        probe.stop()?;
    }
    let mut startup = Vec::with_capacity(config.startup_samples);
    for _ in 0..config.startup_samples {
        let (probe, elapsed) = Probe::spawn()?;
        startup.push(millis(elapsed));
        probe.stop()?;
    }

    let (mut probe, _) = Probe::spawn()?;
    std::thread::sleep(Duration::from_millis(config.idle_settle_ms));
    let mut rss = Vec::with_capacity(config.idle_samples);
    for _ in 0..config.idle_samples {
        rss.push(probe.rss_mib()?);
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut input = Vec::with_capacity(config.input_samples);
    for sequence in 0..config.input_samples {
        input.push(micros(probe.input_round_trip(sequence)?));
    }
    probe.stop()?;

    let (render_unchanged, render_changed, render_fps) = benchmark_render(config.render_samples)?;
    let (lua_noop, lua_batch, snapshot_bytes, effect) =
        benchmark_lua(config.lua_samples, config.effect_samples)?;

    Ok(Report {
        schema: SCHEMA,
        source_revision: source_revision(),
        fixture_revision: config.fixture_revision,
        profile: "release",
        environment: Environment {
            os: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            cpu: cpu_model(),
            logical_cpus: std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
            total_memory_mib: total_memory_mib(),
        },
        conditions: Conditions {
            startup: "warm OS/filesystem cache; process spawn to first input-ready frame",
            idle: format!(
                "canonical empty probe after {} ms settle; /proc VmRSS",
                config.idle_settle_ms
            ),
            input: "parent pipe write timestamp to child frame-flush acknowledgement",
            render: "80-row retained frame at 120x40; unchanged and one changed row",
            lua: "bounded JSON snapshot; exact copied bytes reported; no-op and three-action batch",
            effect: "queued Lua dispatch through local zero-duration timer completion",
        },
        parameters: Parameters {
            warmup_runs: config.warmup_runs,
            startup_samples: config.startup_samples,
            idle_samples: config.idle_samples,
            input_samples: config.input_samples,
            render_samples: config.render_samples,
            lua_samples: config.lua_samples,
            effect_samples: config.effect_samples,
        },
        metrics: Metrics {
            startup_ms: distribution(startup, "ms")?,
            idle_rss_mib: distribution(rss, "MiB")?,
            input_to_frame_us: distribution(input, "us")?,
            retained_render_unchanged_us: render_unchanged,
            retained_render_changed_us: render_changed,
            retained_render_changed_frames_per_second: render_fps,
            lua_dispatch_noop_us: lua_noop,
            lua_dispatch_action_batch_us: lua_batch,
            lua_snapshot_bytes: snapshot_bytes,
            effect_round_trip_us: effect,
        },
    })
}

fn self_test() -> Result<(), Error> {
    let distribution = distribution(vec![5.0, 1.0, 4.0, 2.0, 3.0], "us")?;
    if distribution.min != 1.0
        || distribution.p50 != 3.0
        || distribution.p95 != 5.0
        || distribution.max != 5.0
    {
        return Err(Error::Probe(
            "percentile negative control failed".to_owned(),
        ));
    }
    let config = Config {
        schema: SCHEMA.to_owned(),
        fixture_revision: "self-test".to_owned(),
        warmup_runs: 0,
        startup_samples: 1,
        idle_samples: 1,
        idle_settle_ms: 0,
        input_samples: 1,
        render_samples: 1,
        lua_samples: 1,
        effect_samples: 1,
    };
    validate_config(&config)?;
    let encoded = serde_json::to_string(&config)
        .map_err(|error| Error::Config(format!("self-test serialization: {error}")))?;
    let decoded: Config = serde_json::from_str(&encoded)
        .map_err(|error| Error::Config(format!("self-test parsing: {error}")))?;
    validate_config(&decoded)?;
    println!("{SCHEMA} self-test passed");
    Ok(())
}

fn run() -> Result<(), Error> {
    let mut args = std::env::args_os().skip(1);
    match args
        .next()
        .and_then(|value| value.into_string().ok())
        .as_deref()
    {
        Some("--probe") if args.next().is_none() => probe_main(),
        Some("--self-test") if args.next().is_none() => self_test(),
        Some("--config") => {
            if cfg!(debug_assertions) {
                return Err(Error::DebugBuild);
            }
            let config_path = PathBuf::from(args.next().ok_or(Error::Usage)?);
            let mut output_path = None;
            if let Some(flag) = args.next() {
                if flag != "--output" {
                    return Err(Error::Usage);
                }
                output_path = Some(PathBuf::from(args.next().ok_or(Error::Usage)?));
            }
            if args.next().is_some() {
                return Err(Error::Usage);
            }
            let report = benchmark(read_config(&config_path)?)?;
            let mut bytes = serde_json::to_vec_pretty(&report)
                .map_err(|error| Error::Config(format!("report serialization: {error}")))?;
            bytes.push(b'\n');
            if let Some(path) = output_path {
                fs::write(&path, bytes).map_err(|source| Error::Io { path, source })?;
            } else {
                std::io::stdout()
                    .write_all(&bytes)
                    .map_err(|source| Error::Io {
                        path: PathBuf::from("<stdout>"),
                        source,
                    })?;
            }
            Ok(())
        }
        _ => Err(Error::Usage),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("performance-baseline: {error}");
            ExitCode::FAILURE
        }
    }
}
