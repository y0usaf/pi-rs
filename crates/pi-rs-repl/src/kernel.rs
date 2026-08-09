//! KernelManager: one long-lived Python child per agent scope.
//!
//! Spawns the Python kernel shim, owns its lifecycle, and multiplexes
//! execute/interrupt/snapshot/restore/shutdown against the shim's framed
//! stdio. Per-cell watchdog: timeout -> Interrupt frame (the shim delivers
//! KeyboardInterrupt in-process) -> grace -> SIGINT to the process group ->
//! SIGKILL + respawn. Stale correlation: every request carries an id;
//! respawn bumps the generation and clears pending, so late events from an
//! interrupted or replaced kernel cannot complete a newer request.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::framing::{decode_shim, encode_host, read_frame, write_frame};
use crate::protocol::{ExecuteResult, HostMsg, RestoreResult, ShimMsg, SnapshotResult, WIRE_VERSION};

/// The embedded kernel shim source (shim/kernel-shim.py).
pub const KERNEL_SHIM_SOURCE: &str = include_str!("../shim/kernel-shim.py");

/// Host-side handler for a kernel host_request (e.g. "rlm.run", "goal.complete").
/// Sync in P1; the Lua seam bridges to coroutines. The returned value is the
/// reply payload, passed back verbatim as the host_request result.
pub type HostRequestHandler =
    Arc<dyn Fn(String, serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync>;

/// Per-execution stream callback (stdout/stderr chunks for live UI).
pub type StreamCallback = Arc<dyn Fn(u64, String, String) + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("kernel spawn failed: {0}")]
    Spawn(String),
    #[error("shim failed to become ready: {0}")]
    Ready(String),
    #[error("wire version mismatch: shim {shim}, host {host}")]
    VersionMismatch { shim: u32, host: u32 },
    #[error("kernel died: {0}")]
    Died(String),
    #[error("cell {id} exceeded watchdog budget ({watchdog_ms} ms)")]
    Watchdog { id: u64, watchdog_ms: u64 },
    #[error("{0}")]
    Other(String),
}

#[derive(Clone)]
pub struct KernelConfig {
    /// Python interpreter with IPython, dill, nest-asyncio and the vendored
    /// prime-agent-runtime on its path.
    pub python: PathBuf,
    pub cwd: Option<PathBuf>,
    /// Extra environment (appended over the inherited environment).
    pub env: Vec<(String, String)>,
    /// Per-cell execution budget. Default 300_000 ms.
    pub watchdog_ms: u64,
    /// Grace after Interrupt before SIGINT to the process group. Default 1000 ms.
    pub interrupt_grace_ms: u64,
    /// Handler for host_request frames (rlm.run, find_models, harness ops, ...).
    pub host_handler: Option<HostRequestHandler>,
    /// Live stream callback for stdout/stderr chunks.
    pub on_stream: Option<StreamCallback>,
}

impl Default for KernelConfig {
    fn default() -> Self {
        Self {
            python: PathBuf::from("python3"),
            cwd: None,
            env: Vec::new(),
            watchdog_ms: 300_000,
            interrupt_grace_ms: 1_000,
            host_handler: None,
            on_stream: None,
        }
    }
}

struct Pending {
    done: oneshot::Sender<Result<serde_json::Value, String>>,
}

struct Inner {
    config: KernelConfig,
    shim_path: PathBuf,
    dead: AtomicBool,
    pending: Mutex<HashMap<u64, Pending>>,
    next_id: AtomicU64,
    stdin: Mutex<Option<ChildStdin>>,
    reader: Mutex<Option<JoinHandle<()>>>,
    child_pid: Mutex<Option<u32>>,
}

#[derive(Clone)]
pub struct KernelManager {
    inner: Arc<Inner>,
}

impl KernelManager {
    /// Spawn a kernel shim and wait for its ready frame.
    pub async fn spawn(config: KernelConfig) -> Result<Self, KernelError> {
        let shim_path = std::env::temp_dir().join(format!(
            "pi-rs-repl-shim-{}-{}.py",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&shim_path, KERNEL_SHIM_SOURCE)
            .map_err(|e| KernelError::Spawn(format!("write shim: {e}")))?;

        let inner = Arc::new(Inner {
            config,
            shim_path: shim_path.clone(),
            dead: AtomicBool::new(false),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            stdin: Mutex::new(None),
            reader: Mutex::new(None),
            child_pid: Mutex::new(None),
        });

        let mgr = Self { inner };
        mgr.spawn_child().await?;
        Ok(mgr)
    }

    /// Spawn the python child (initial or respawned) and wait for ready.
    async fn spawn_child(&self) -> Result<(), KernelError> {
        let mut cmd = Command::new(&self.inner.config.python);
        cmd.arg(&self.inner.shim_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &self.inner.config.cwd {
            cmd.current_dir(cwd);
        }
        for (k, v) in &self.inner.config.env {
            cmd.env(k, v);
        }
        // New process group so the watchdog can SIGINT the whole group.
        cmd.process_group(0);

        let mut child = cmd
            .spawn()
            .map_err(|e| KernelError::Spawn(e.to_string()))?;
        let pid = child.id().ok_or_else(|| KernelError::Spawn("child has no pid".into()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| KernelError::Spawn("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| KernelError::Spawn("no stdout".into()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| KernelError::Spawn("no stderr".into()))?;

        *self.inner.stdin.lock().await = Some(stdin);
        *self.inner.child_pid.lock().await = Some(pid);
        self.inner.dead.store(false, Ordering::SeqCst);

        // The shim's own stderr is host-visible diagnostics; drain it so a
        // noisy shim cannot block on a full pipe.
        tokio::spawn(async move {
            let mut r = BufReader::new(&mut stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => eprintln!("[pi-rs-repl:kernel] {}", line.trim_end()),
                }
            }
        });

        // Reader task owns shim stdout; the ready frame resolves here.
        let (ready_tx, ready_rx) = oneshot::channel();
        let mut ready_tx = Some(ready_tx);
        let inner = self.inner.clone();
        let reader = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_frame(&mut reader).await {
                    Ok(Some(buf)) => match decode_shim(&buf) {
                        Ok(ShimMsg::Ready { v, error }) => {
                            if let Some(tx) = ready_tx.take() {
                                let _ = tx.send(if v != WIRE_VERSION {
                                    Err(KernelError::VersionMismatch { shim: v, host: WIRE_VERSION })
                                } else if let Some(e) = error {
                                    Err(KernelError::Ready(e))
                                } else {
                                    Ok(())
                                });
                            }
                        }
                        Ok(msg) => inner.handle_shim_msg(msg).await,
                        Err(e) => eprintln!("[pi-rs-repl] bad frame from shim: {e}"),
                    },
                    Ok(None) => {
                        inner.mark_dead("stdout EOF".to_string());
                        break;
                    }
                    Err(e) => {
                        inner.mark_dead(format!("read error: {e}"));
                        break;
                    }
                }
            }
        });
        *self.inner.reader.lock().await = Some(reader);

        // Wait for the ready frame with a startup timeout.
        match tokio::time::timeout(Duration::from_secs(60), ready_rx).await {
            Ok(Ok(Ok(()))) => Ok(()),

            Ok(Ok(Err(e))) => {
                let _ = child.kill().await;
                self.inner.mark_dead("ready failed".into());
                eprintln!("[pi-rs-repl] ready failed: {e}");
                Err(e)
            }
            Ok(Err(_)) => {
                let _ = child.kill().await;
                self.inner.mark_dead("reader dropped".into());
                eprintln!("[pi-rs-repl] reader dropped before ready (pid {pid})");
                Err(KernelError::Spawn("shim exited before ready".into()))
            }
            Err(_) => {
                let _ = child.kill().await;
                self.inner.mark_dead("ready timeout".into());
                eprintln!("[pi-rs-repl] ready timeout after 60s (pid {pid})");
                Err(KernelError::Ready("startup timed out after 60s".into()))
            }
        }
    }

    /// Send one host frame to the shim (serialized).
    async fn send(&self, msg: &HostMsg) -> Result<(), KernelError> {
        if self.inner.dead.load(Ordering::SeqCst) {
            return Err(KernelError::Died("kernel dead; respawn required".into()));
        }
        let payload = encode_host(msg).map_err(|e| KernelError::Other(e.to_string()))?;
        let mut stdin = self.inner.stdin.lock().await;
        let Some(stdin) = stdin.as_mut() else {
            return Err(KernelError::Died("no stdin (kernel dead)".into()));
        };
        write_frame(stdin, &payload)
            .await
            .map_err(|e| KernelError::Died(e.to_string()))
    }

    /// Execute one cell.
    pub async fn execute(
        &self,
        code: &str,
        max_chars: Option<u64>,
    ) -> Result<ExecuteResult, KernelError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(id, Pending { done: tx });

        self.send(&HostMsg::Execute {
            v: WIRE_VERSION,
            id,
            code: code.to_string(),
            max_chars: max_chars.unwrap_or(65_536),
        })
        .await?;

        match tokio::time::timeout(
            Duration::from_millis(self.inner.config.watchdog_ms),
            rx,
        )
        .await
        {
            Ok(Ok(Ok(value))) => serde_json::from_value(value)
                .map_err(|e| KernelError::Other(format!("bad result payload: {e}"))),
            Ok(Ok(Err(e))) => Err(KernelError::Other(e)),
            Ok(Err(_)) => Err(KernelError::Other("cell oneshot dropped".into())),
            Err(_) => {
                self.watchdog_kill().await;
                Err(KernelError::Watchdog { id, watchdog_ms: self.inner.config.watchdog_ms })
            }
        }
    }

    /// Interrupt the running cell.
    pub async fn interrupt(&self) -> Result<(), KernelError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        self.send(&HostMsg::Interrupt { v: WIRE_VERSION, id }).await
    }

    /// Watchdog path: interrupt, wait grace, SIGINT the group, then SIGKILL +
    /// respawn.
    pub async fn watchdog_kill(&self) {
        let _ = self.interrupt().await;
        tokio::time::sleep(Duration::from_millis(self.inner.config.interrupt_grace_ms)).await;
        let pid = self.inner.child_pid.lock().await.as_ref().copied();
        if let Some(pid) = pid {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGINT);
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = self.respawn().await;
    }

    /// Kill the current child and start a fresh one (same config).
    ///
    /// Lock discipline: never hold a mutex guard across an await or nest a
    /// second acquire of the same mutex inside a guard's scope (tokio Mutex
    /// is not reentrant; an if-let scrutinee guard lives through the whole
    /// block and self-deadlocks on a nested acquire).
    pub async fn respawn(&self) -> Result<(), KernelError> {
        self.inner.mark_dead("respawn".into());
        if let Some(reader) = self.inner.reader.lock().await.take() {
            reader.abort();
        }
        let pid = self.inner.child_pid.lock().await.take();
        if let Some(pid) = pid {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        self.inner.next_id.store(1, Ordering::SeqCst);
        self.inner.pending.lock().await.clear();
        self.spawn_child().await
    }

    /// Snapshot the kernel user namespace to disk (dill payload + manifest).
    pub async fn snapshot(
        &self,
        path: &Path,
        manifest_path: &Path,
        max_bytes: Option<u64>,
    ) -> Result<SnapshotResult, KernelError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(id, Pending { done: tx });
        self.send(&HostMsg::Snapshot {
            v: WIRE_VERSION,
            id,
            path: path.to_string_lossy().into_owned(),
            manifest_path: manifest_path.to_string_lossy().into_owned(),
            max_bytes: max_bytes.unwrap_or(256 * 1024 * 1024),
        })
        .await?;
        match tokio::time::timeout(Duration::from_secs(120), rx).await {
            Ok(Ok(Ok(value))) => serde_json::from_value(value)
                .map_err(|e| KernelError::Other(format!("bad snapshot payload: {e}"))),
            Ok(Ok(Err(e))) => Err(KernelError::Other(e)),
            Ok(Err(_)) => Err(KernelError::Other("snapshot oneshot dropped".into())),
            Err(_) => Err(KernelError::Watchdog { id, watchdog_ms: 120_000 }),
        }
    }

    /// Restore a dill snapshot into the kernel user namespace.
    pub async fn restore(&self, path: &Path) -> Result<RestoreResult, KernelError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(id, Pending { done: tx });
        self.send(&HostMsg::Restore { v: WIRE_VERSION, id, path: path.to_string_lossy().into_owned() })
            .await?;
        match tokio::time::timeout(Duration::from_secs(120), rx).await {
            Ok(Ok(Ok(value))) => serde_json::from_value(value)
                .map_err(|e| KernelError::Other(format!("bad restore payload: {e}"))),
            Ok(Ok(Err(e))) => Err(KernelError::Other(e)),
            Ok(Err(_)) => Err(KernelError::Other("restore oneshot dropped".into())),
            Err(_) => Err(KernelError::Watchdog { id, watchdog_ms: 120_000 }),
        }
    }

    /// Dispose the kernel: graceful shutdown frame, then kill; remove the
    /// shim temp file. Idempotent.
    pub async fn shutdown(&self) {
        if self.inner.dead.load(Ordering::SeqCst) {
            return;
        }
        let _ = self
            .send(&HostMsg::Shutdown { v: WIRE_VERSION, id: self.inner.next_id.fetch_add(1, Ordering::SeqCst) })
            .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.inner.mark_dead("shutdown".into());
        if let Some(reader) = self.inner.reader.lock().await.take() {
            reader.abort();
        }
        if let Some(pid) = *self.inner.child_pid.lock().await {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        let _ = std::fs::remove_file(&self.inner.shim_path);
    }

    pub fn is_dead(&self) -> bool {
        self.inner.dead.load(Ordering::SeqCst)
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.shim_path);
    }
}

impl Inner {
    async fn handle_shim_msg(&self, msg: ShimMsg) {
        match msg {
            ShimMsg::Ready { .. } => {}
            ShimMsg::Stream { id, name, chunk, .. } => {
                if let Some(cb) = &self.config.on_stream {
                    cb(id, name, chunk);
                }
            }
            ShimMsg::Result { id, ok, value, error, .. } => {
                if let Some(p) = self.pending.lock().await.remove(&id) {
                    let _ = p.done.send(if ok {
                        Ok(value)
                    } else {
                        Err(error.unwrap_or_else(|| "shim reported error".into()))
                    });
                }
            }
            ShimMsg::HostRequest { req_id, kind, payload, .. } => {
                // Reply shape matches the shim's _stdio_host_request: the
                // payload dict with an inline status the shim strips.
                let reply = match &self.config.host_handler {
                    Some(handler) => match handler(kind, payload) {
                        Ok(payload) => {
                            let mut reply = payload.as_object().cloned().unwrap_or_default();
                            reply.insert("status".to_string(), serde_json::json!("ok"));
                            serde_json::Value::Object(reply)
                        }
                        Err(e) => serde_json::json!({ "status": "error", "error": e }),
                    },
                    None => serde_json::json!({ "status": "error", "error": "no host handler registered" }),
                };
                let status = reply["status"].as_str().unwrap_or("error").to_string();
                let msg = HostMsg::HostResponse { v: WIRE_VERSION, req_id, status, payload: reply };
                let mut stdin = self.stdin.lock().await;
                if let (Some(stdin), Ok(payload)) = (stdin.as_mut(), encode_host(&msg)) {
                    let _ = write_frame(stdin, &payload).await;
                }
            }
            ShimMsg::SnapshotData { id, ok, value, error, .. } => {
                if let Some(p) = self.pending.lock().await.remove(&id) {
                    let _ = p.done.send(if ok {
                        Ok(value.unwrap_or(serde_json::Value::Null))
                    } else {
                        Err(error.unwrap_or_else(|| "snapshot failed".into()))
                    });
                }
            }
            ShimMsg::RestoreData { id, ok, value, error, .. } => {
                if let Some(p) = self.pending.lock().await.remove(&id) {
                    let _ = p.done.send(if ok {
                        Ok(value.unwrap_or(serde_json::Value::Null))
                    } else {
                        Err(error.unwrap_or_else(|| "restore failed".into()))
                    });
                }
            }
        }
    }

    fn mark_dead(&self, reason: String) {
        if self.dead.swap(true, Ordering::SeqCst) {
            return;
        }
        eprintln!("[pi-rs-repl] kernel died: {reason}");
    }
}
