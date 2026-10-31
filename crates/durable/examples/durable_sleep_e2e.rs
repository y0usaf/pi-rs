use std::error::Error;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pi_durable::ai::providers::faux::{Faux, FauxOptions, FauxResponse};
use pi_durable::cancel::CancelToken;
use pi_durable::harness::agent::{AgentChange, Change};
use pi_durable::harness::harness::Harness;
use pi_durable::harness::registry::Registry;
use pi_durable::harness::scheduler::Inspected;
use pi_durable::harness::submissions::{SubmissionDraft, WhenBusy};
use pi_durable::harness::timers::{Disarm, TimerCallback};
use pi_durable::harness::types::{
    Extension, HarnessOptions, HarnessSettings, ModelRef, RetryOverrides, RootOptions,
};
use pi_durable::storage::sqlite::SqliteStorage;
use pi_durable::storage::sqlite::migrations::{SQLITE_MIGRATIONS, apply_migrations};
use rusqlite::Connection;
use serde_json::{Value, json};

type Out<T> = std::result::Result<T, Box<dyn Error>>;

const BACKOFF_MS: u64 = 1500;

fn check(condition: bool, message: &str) -> Out<()> {
    if condition {
        return Ok(());
    }
    Err(message.to_owned().into())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn assistant(content: Value, stop_reason: &str, error: Option<&str>) -> Out<FauxResponse> {
    let mut message = json!({
        "role": "assistant",
        "content": content,
        "api": "faux",
        "provider": "faux",
        "model": "faux-1",
        "usage": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
        "stopReason": stop_reason,
        "timestamp": now_ms(),
    });
    if let Some(error) = error {
        message["errorMessage"] = json!(error);
    }
    Ok(FauxResponse::Message(Box::new(serde_json::from_value(
        message,
    )?)))
}

struct Timer {
    at: u64,
    seq: u64,
    callback: TimerCallback,
    fired: Arc<AtomicBool>,
    background: bool,
}

/// The harness clock and timers this check drives by hand: a sleeping task resumes when both move.
#[derive(Clone)]
struct FakeClock {
    now: Arc<AtomicU64>,
    seq: Arc<AtomicU64>,
    timers: Arc<Mutex<Vec<Timer>>>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            now: Arc::new(AtomicU64::new(1000)),
            seq: Arc::new(AtomicU64::new(0)),
            timers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn clock(&self) -> Arc<dyn Fn() -> u64 + Send + Sync> {
        let now = Arc::clone(&self.now);
        Arc::new(move || now.load(Ordering::SeqCst))
    }

    fn set_timer(&self) -> pi_durable::harness::timers::SetTimer {
        let clock = self.clone();
        Arc::new(
            move |callback: TimerCallback, delay_ms: u64, background: bool| {
                let at = clock.now.load(Ordering::SeqCst) + delay_ms;
                let seq = clock.seq.fetch_add(1, Ordering::SeqCst);
                let fired = Arc::new(AtomicBool::new(false));
                let disarm = Arc::clone(&fired);
                clock.timers.lock().expect("timers").push(Timer {
                    at,
                    seq,
                    callback,
                    fired: Arc::clone(&fired),
                    background,
                });
                Some(Box::new(move || disarm.store(true, Ordering::SeqCst)) as Disarm)
            },
        )
    }

    fn armed(&self) -> bool {
        self.timers
            .lock()
            .expect("timers")
            .iter()
            .any(|timer| !timer.background && !timer.fired.load(Ordering::SeqCst))
    }

    fn advance(&self, ms: u64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
        loop {
            let now = self.now.load(Ordering::SeqCst);
            let due = {
                let mut timers = self.timers.lock().expect("timers");
                let index = timers
                    .iter()
                    .enumerate()
                    .filter(|(_, timer)| {
                        !timer.background && !timer.fired.load(Ordering::SeqCst) && timer.at <= now
                    })
                    .min_by_key(|(_, timer)| (timer.at, timer.seq))
                    .map(|(index, _)| index);
                index.map(|index| timers.remove(index))
            };
            let Some(timer) = due else {
                return;
            };
            timer.fired.store(true, Ordering::SeqCst);
            (timer.callback)();
        }
    }
}

fn registry() -> Arc<Registry> {
    let registry = Arc::new(Registry::default());
    registry
        .install(Extension {
            name: "empty".to_owned(),
            tools: vec![],
            sections: vec![],
            wraps: vec![],
            hooks: vec![],
            tasks: vec![],
        })
        .expect("install the built-in tasks");
    registry
}

fn harness_options(faux: Arc<Faux>, clock: &FakeClock) -> HarnessOptions {
    HarnessOptions {
        models: faux,
        registry: registry(),
        settings: Some(HarnessSettings {
            retry: Some(RetryOverrides {
                enabled: Some(true),
                max_retries: Some(2),
                base_delay_ms: Some(BACKOFF_MS),
                max_agent_delay_ms: None,
            }),
            ..HarnessSettings::default()
        }),
        conversation_created: None,
        now: Some(clock.clock()),
        env: None,
        set_timer: Some(clock.set_timer()),
        on_report: Some(Arc::new(|error| eprintln!("report: {error}"))),
    }
}

fn root(
    harness: &Harness,
    cancel: &CancelToken,
) -> Out<pi_durable::harness::harness::Conversation> {
    Ok(harness.root(
        RootOptions {
            agent: Some(AgentChange {
                model: Change::Set(ModelRef {
                    provider: "faux".to_owned(),
                    model_id: "faux-1".to_owned(),
                }),
                ..Default::default()
            }),
            init: None,
        },
        cancel,
    )?)
}

fn sleeping(harness: &Harness, cancel: &CancelToken) -> Out<Option<u64>> {
    let inspected = harness.inspect(cancel)?;
    Ok(inspected.tasks.iter().find_map(|(_, state)| match state {
        Inspected::Sleeping { until } => Some(*until),
        _ => None,
    }))
}

fn wait_for(what: &str, ready: impl Fn() -> Out<bool>) -> Out<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready()? {
        if Instant::now() > deadline {
            return Err(format!("{what} did not happen within 30s").into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

/// A version 1 store is what an older build wrote: the sleeping status is not in its CHECK
/// constraint. Opening it through the storage migrates the file in place.
fn stored_version(connection: &Connection) -> Out<u64> {
    Ok(connection.query_row(
        "SELECT version FROM durable_schema WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?)
}

fn create_v1(path: &std::path::Path) -> Out<()> {
    let mut connection = Connection::open(path)?;
    apply_migrations(&mut connection, &SQLITE_MIGRATIONS[..1])?;
    Ok(())
}

fn main() -> Out<()> {
    let dir = std::env::temp_dir().join(format!("pi-durable-sleep-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("sleep.db");
    let _ = std::fs::remove_file(&path);
    let cancel = CancelToken::new();
    create_v1(&path)?;
    {
        let connection = Connection::open(&path)?;
        check(
            stored_version(&connection)? == 1,
            "a store from an older build is at schema version 1",
        )?;
        check(
            connection
                .execute(
                    "INSERT INTO tasks (id, conversation_id, kind, status, abort_requested, background, record)
                     VALUES (1, 1, 'pi.generation', 'sleeping', 0, 0, '{}')",
                    [],
                )
                .is_err(),
            "a version 1 store rejects the sleeping status",
        )?;
    }

    let clock = FakeClock::new();
    let faux = Arc::new(Faux::new(FauxOptions::default()));
    faux.set_responses(vec![assistant(json!([]), "error", Some("503 overloaded"))?]);
    let harness = Harness::open(
        Arc::new(SqliteStorage::open(
            path.to_str().ok_or("path")?,
            &Default::default(),
        )?),
        harness_options(Arc::clone(&faux), &clock),
    )?;
    let conversation = root(&harness, &cancel)?;
    conversation.submit(
        SubmissionDraft::Input {
            request_id: None,
            content: serde_json::from_value(json!("hello"))?,
            when_busy: WhenBusy::default(),
        },
        &cancel,
    )?;
    wait_for("a sleeping generation task", || {
        Ok(sleeping(&harness, &cancel)?.is_some())
    })?;
    let until = sleeping(&harness, &cancel)?.ok_or("no sleep")?;
    check(
        until == 1000 + BACKOFF_MS,
        "the sleep deadline is the retry backoff on the harness clock",
    )?;
    check(
        clock.armed(),
        "the scheduler armed a timer that keeps the process alive",
    )?;
    check(
        harness
            .inspect(&cancel)?
            .tasks
            .iter()
            .all(|(_, state)| !matches!(state, Inspected::Running)),
        "a sleeping task holds no invocation",
    )?;

    let closed = Instant::now();
    harness.close(&cancel)?;
    check(
        closed.elapsed() < Duration::from_secs(2),
        "closing a harness with a sleeping task does not wait for its deadline",
    )?;

    let clock = FakeClock::new();
    let faux = Arc::new(Faux::new(FauxOptions::default()));
    faux.set_responses(vec![assistant(
        json!([{"type": "text", "text": "after the restart"}]),
        "stop",
        None,
    )?]);
    let harness = Harness::open(
        Arc::new(SqliteStorage::open(
            path.to_str().ok_or("path")?,
            &Default::default(),
        )?),
        harness_options(Arc::clone(&faux), &clock),
    )?;
    harness.resume()?;
    let conversation = root(&harness, &cancel)?;
    wait_for("a sleeping generation task after the restart", || {
        Ok(sleeping(&harness, &cancel)?.is_some())
    })?;
    check(
        clock.armed(),
        "the reopened harness armed the wake timer for the stored deadline",
    )?;
    clock.advance(BACKOFF_MS + 500);
    harness.wait_for_idle(&cancel)?;
    let page = conversation.entries(
        pi_durable::types::EntryQuery {
            conversation_id: conversation.id(),
            min_entry_id: None,
            max_entry_id: None,
            order: Some(pi_durable::types::ScanOrder::Ascending),
        },
        1000,
        None,
        &cancel,
    )?;
    let last = page.items.last().ok_or("no entries")?.model.clone();
    check(
        serde_json::to_string(&last)?.contains("after the restart"),
        "the task resumed at its checkpoint and finished the retry",
    )?;
    check(
        sleeping(&harness, &cancel)?.is_none(),
        "the task is no longer asleep",
    )?;
    harness.close(&cancel)?;

    let connection = Connection::open(&path)?;
    check(
        stored_version(&connection)? == SQLITE_MIGRATIONS.last().ok_or("migrations")?.version,
        "storing a sleep upgraded the version 1 store in place",
    )?;

    println!(
        "durable sleep e2e ok: slept {BACKOFF_MS}ms without an invocation, resumed after a restart, and upgraded a version 1 store in place"
    );
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
