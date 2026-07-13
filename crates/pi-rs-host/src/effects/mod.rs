//! Scope-owned asynchronous OS effects.
//!
//! Every operation crosses one bounded typed queue. Requests carry explicit
//! timeout and cancellation semantics; stream producers use bounded channels
//! and retain their package resource lease until completion.

mod clipboard;
mod crypto;
mod fs;
mod http;
mod process;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::kernel::{CancellationToken, Control, ResourceId, ScopeId};

pub use clipboard::{ClipboardImage, ClipboardResult, extension_for_image_mime_type};
pub use fs::{FileStat, FsResult};
pub use http::{HttpRequest, HttpResponse, HttpStream};
pub use process::{ProcessEvent, ProcessOutput, ProcessOutputKind, ProcessRequest, ProcessStream};

pub const REQUEST_QUEUE_CAPACITY: usize = 32;
pub const MAX_IN_FLIGHT: usize = 16;
pub const DEFAULT_STREAM_CAPACITY: usize = 8;
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTimeout {
    After(Duration),
    Disabled,
}

impl EffectTimeout {
    fn wait(self) -> impl Future<Output = ()> {
        async move {
            match self {
                Self::After(duration) => tokio::time::sleep(duration).await,
                Self::Disabled => std::future::pending().await,
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct EffectOptions {
    pub timeout: EffectTimeout,
    pub stream_capacity: usize,
    pub max_output_bytes: usize,
}

impl EffectOptions {
    #[must_use]
    pub fn bounded(timeout: Duration) -> Self {
        Self {
            timeout: EffectTimeout::After(timeout),
            stream_capacity: DEFAULT_STREAM_CAPACITY,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    #[must_use]
    pub fn long_lived() -> Self {
        Self {
            timeout: EffectTimeout::Disabled,
            stream_capacity: DEFAULT_STREAM_CAPACITY,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    fn validate(&self) -> Result<(), EffectError> {
        if self.stream_capacity == 0 || self.stream_capacity > REQUEST_QUEUE_CAPACITY {
            return Err(EffectError::Invalid(format!(
                "stream_capacity must be in 1..={REQUEST_QUEUE_CAPACITY}"
            )));
        }
        if self.max_output_bytes == 0 {
            return Err(EffectError::Invalid(
                "max_output_bytes must be greater than zero".to_owned(),
            ));
        }
        if matches!(self.timeout, EffectTimeout::After(duration) if duration.is_zero()) {
            return Err(EffectError::Invalid(
                "timeout must be greater than zero or explicitly disabled".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum FsRequest {
    Read { path: String, bytes: bool },
    Write { path: String, contents: Vec<u8> },
    Append { path: String, contents: Vec<u8> },
    Exists { path: String },
    ReadDir { path: String },
    Stat { path: String },
    Mkdir { path: String },
    Realpath { path: String },
    RemoveFile { path: String },
    CreateTempFile { prefix: String, contents: Vec<u8> },
}

#[derive(Debug, Clone)]
pub enum ClipboardRequest {
    ReadImage {
        env: std::collections::HashMap<String, String>,
        platform: String,
    },
    WriteText {
        text: String,
        env: std::collections::HashMap<String, String>,
        platform: String,
    },
}

#[derive(Debug, Clone)]
pub enum CryptoRequest {
    Sha256(Vec<u8>),
    RandomBytes(usize),
    RandomUuid,
}

#[derive(Debug)]
pub enum EffectRequest {
    Fs(FsRequest, EffectOptions),
    Process(ProcessRequest),
    Http(HttpRequest),
    Timer(Duration, EffectOptions),
    Clipboard(ClipboardRequest, EffectOptions),
    Crypto(CryptoRequest, EffectOptions),
}

impl EffectRequest {
    fn options(&self) -> &EffectOptions {
        match self {
            Self::Fs(_, options)
            | Self::Timer(_, options)
            | Self::Clipboard(_, options)
            | Self::Crypto(_, options) => options,
            Self::Process(request) => &request.options,
            Self::Http(request) => &request.options,
        }
    }
}

#[derive(Debug)]
pub enum EffectResult {
    Fs(fs::FsResult),
    Process(ProcessStream),
    Http(HttpResponse),
    Timer,
    Clipboard(clipboard::ClipboardResult),
    Crypto(Vec<u8>),
    Uuid(String),
}

#[derive(Debug, thiserror::Error)]
pub enum EffectError {
    #[error("effect cancelled")]
    Cancelled,
    #[error("effect timed out")]
    Timeout,
    #[error("effect output exceeded {0} bytes")]
    OutputLimit(usize),
    #[error("invalid effect request: {0}")]
    Invalid(String),
    #[error("effect queue is unavailable")]
    QueueClosed,
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Http(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectStats {
    pub active: usize,
    pub queued: usize,
    pub peak_active: usize,
    pub peak_queued: usize,
}

#[derive(Debug, Default)]
struct Stats {
    active: AtomicUsize,
    queued: AtomicUsize,
    peak_active: AtomicUsize,
    peak_queued: AtomicUsize,
    scopes: Mutex<BTreeMap<ScopeId, usize>>,
    settled: tokio::sync::Notify,
}

fn update_peak(peak: &AtomicUsize, value: usize) {
    let mut observed = peak.load(Ordering::Relaxed);
    while value > observed {
        match peak.compare_exchange_weak(observed, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => observed = next,
        }
    }
}

pub(crate) struct ResourceLease {
    control: Arc<Control>,
    stats: Arc<Stats>,
    scope: ScopeId,
    resource: ResourceId,
}

impl ResourceLease {
    fn new(control: Arc<Control>, stats: Arc<Stats>, scope: ScopeId) -> Result<Self, EffectError> {
        let resource = control
            .register_resource(scope)
            .map_err(|error| EffectError::Invalid(error.to_string()))?;
        let active = stats.active.fetch_add(1, Ordering::AcqRel) + 1;
        update_peak(&stats.peak_active, active);
        *stats
            .scopes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(scope)
            .or_default() += 1;
        Ok(Self {
            control,
            stats,
            scope,
            resource,
        })
    }
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        self.control.release_resource(self.scope, self.resource);
        self.stats.active.fetch_sub(1, Ordering::AcqRel);
        let mut scopes = self
            .stats
            .scopes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(active) = scopes.get_mut(&self.scope) {
            *active -= 1;
            if *active == 0 {
                scopes.remove(&self.scope);
            }
        }
        drop(scopes);
        self.stats.settled.notify_waiters();
    }
}

#[derive(Clone)]
pub(crate) struct RequestContext {
    pub scope: CancellationToken,
    pub request: CancellationToken,
}

impl RequestContext {
    pub async fn cancelled(&self) {
        tokio::select! {
            () = self.scope.cancelled() => (),
            () = self.request.cancelled() => (),
        }
    }
}

struct Envelope {
    request: EffectRequest,
    context: RequestContext,
    lease: ResourceLease,
    reply: oneshot::Sender<Result<EffectResult, EffectError>>,
}

#[derive(Clone)]
pub(crate) struct EffectHub {
    tx: mpsc::Sender<Envelope>,
    control: Arc<Control>,
    stats: Arc<Stats>,
}

pub(crate) struct EffectRunner {
    rx: mpsc::Receiver<Envelope>,
    stats: Arc<Stats>,
}

impl EffectHub {
    pub(crate) fn new(control: Arc<Control>) -> (Self, EffectRunner) {
        let (tx, rx) = mpsc::channel(REQUEST_QUEUE_CAPACITY);
        let stats = Arc::new(Stats::default());
        (
            Self {
                tx,
                control,
                stats: Arc::clone(&stats),
            },
            EffectRunner { rx, stats },
        )
    }

    pub(crate) fn scope(&self, lua: &mlua::Lua) -> mlua::Result<ScopeId> {
        crate::kernel_api::scope_for_current_entry(lua)
    }

    pub(crate) async fn request(
        &self,
        scope: ScopeId,
        request: EffectRequest,
        cancellation: CancellationToken,
    ) -> Result<EffectResult, EffectError> {
        request.options().validate()?;
        let scope_token = self
            .control
            .token(scope)
            .map_err(|error| EffectError::Invalid(error.to_string()))?;
        let context = RequestContext {
            scope: scope_token.clone(),
            request: cancellation.clone(),
        };
        let permit = tokio::select! {
            permit = self.tx.reserve() => permit.map_err(|_| EffectError::QueueClosed)?,
            () = scope_token.cancelled() => return Err(EffectError::Cancelled),
            () = cancellation.cancelled() => return Err(EffectError::Cancelled),
        };
        let lease = ResourceLease::new(Arc::clone(&self.control), Arc::clone(&self.stats), scope)?;
        let (reply, rx) = oneshot::channel();
        let queued = self.stats.queued.fetch_add(1, Ordering::AcqRel) + 1;
        update_peak(&self.stats.peak_queued, queued);
        permit.send(Envelope {
            request,
            context,
            lease,
            reply,
        });
        tokio::select! {
            result = rx => result.map_err(|_| EffectError::QueueClosed)?,
            () = scope_token.cancelled() => {
                cancellation.cancel();
                Err(EffectError::Cancelled)
            }
            () = cancellation.cancelled() => Err(EffectError::Cancelled),
        }
    }

    pub(crate) fn stats(&self) -> EffectStats {
        EffectStats {
            active: self.stats.active.load(Ordering::Acquire),
            queued: self.stats.queued.load(Ordering::Acquire),
            peak_active: self.stats.peak_active.load(Ordering::Acquire),
            peak_queued: self.stats.peak_queued.load(Ordering::Acquire),
        }
    }

    pub(crate) async fn settle_scope(&self, scope: ScopeId) {
        let settle = async {
            loop {
                let notified = self.stats.settled.notified();
                let active = self
                    .stats
                    .scopes
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .contains_key(&scope);
                if !active {
                    return;
                }
                notified.await;
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(2), settle).await;
    }
}

impl EffectRunner {
    pub(crate) fn start(mut self, runtime: &tokio::runtime::Runtime) {
        let semaphore = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
        runtime.spawn(async move {
            while let Some(envelope) = self.rx.recv().await {
                self.stats.queued.fetch_sub(1, Ordering::AcqRel);
                let Ok(permit) = Arc::clone(&semaphore).acquire_owned().await else {
                    break;
                };
                tokio::spawn(async move {
                    let _permit = permit;
                    let Envelope {
                        request,
                        context,
                        lease,
                        reply,
                    } = envelope;
                    let result = execute(request, context, lease).await;
                    let _ = reply.send(result);
                });
            }
        });
    }
}

async fn execute(
    request: EffectRequest,
    context: RequestContext,
    lease: ResourceLease,
) -> Result<EffectResult, EffectError> {
    match request {
        EffectRequest::Process(request) => process::start(request, context, Some(lease))
            .await
            .map(EffectResult::Process),
        EffectRequest::Http(request) => http::start(request, context, lease)
            .await
            .map(EffectResult::Http),
        EffectRequest::Fs(request, options) => {
            run_bounded(options.timeout, context, fs::execute(request))
                .await
                .map(EffectResult::Fs)
        }
        EffectRequest::Timer(duration, options) => {
            run_bounded(options.timeout, context, async move {
                tokio::time::sleep(duration).await;
                Ok(EffectResult::Timer)
            })
            .await
        }
        EffectRequest::Clipboard(request, options) => run_bounded(
            options.timeout,
            context.clone(),
            clipboard::execute(request, context),
        )
        .await
        .map(EffectResult::Clipboard),
        EffectRequest::Crypto(request, options) => {
            run_bounded(options.timeout, context, crypto::execute(request)).await
        }
    }
}

async fn run_bounded<T>(
    timeout: EffectTimeout,
    context: RequestContext,
    operation: impl Future<Output = Result<T, EffectError>>,
) -> Result<T, EffectError> {
    tokio::select! {
        result = operation => result,
        () = context.cancelled() => Err(EffectError::Cancelled),
        () = timeout.wait() => Err(EffectError::Timeout),
    }
}

pub(crate) fn cancellation() -> CancellationToken {
    CancellationToken::new()
}

pub(crate) fn lua_error(error: EffectError) -> mlua::Error {
    mlua::Error::runtime(error.to_string())
}

pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table, hub: EffectHub) -> mlua::Result<()> {
    let sleep_hub = hub.clone();
    pi.set(
        "sleep",
        lua.create_async_function(
            move |lua, (milliseconds, signal): (u64, Option<mlua::AnyUserData>)| {
                let hub = sleep_hub.clone();
                async move {
                    let signal = signal
                        .map(|signal| {
                            signal
                                .borrow::<crate::ai::LuaAbortSignal>()
                                .map(|signal| signal.0.clone())
                        })
                        .transpose()?;
                    let scope = hub.scope(&lua)?;
                    let cancellation = cancellation();
                    let duration = Duration::from_millis(milliseconds);
                    let request = hub.request(
                        scope,
                        EffectRequest::Timer(
                            duration,
                            EffectOptions::bounded(duration.saturating_add(Duration::from_secs(1))),
                        ),
                        cancellation.clone(),
                    );
                    let result = if let Some(signal) = signal {
                        tokio::select! {
                            result = request => result,
                            () = signal.aborted() => {
                                cancellation.cancel();
                                return Err(mlua::Error::runtime("sleep aborted"));
                            }
                        }
                    } else {
                        request.await
                    };
                    match result {
                        Ok(EffectResult::Timer) => Ok(()),
                        Err(EffectError::Cancelled) => {
                            Err(mlua::Error::runtime(crate::error::CANCEL_MARKER))
                        }
                        Err(error) => Err(lua_error(error)),
                        Ok(_) => Err(mlua::Error::runtime(
                            "timer effect returned the wrong result",
                        )),
                    }
                }
            },
        )?,
    )?;

    let crypto = lua.create_table()?;
    let hash_hub = hub.clone();
    crypto.set(
        "sha256",
        lua.create_async_function(move |lua, bytes: mlua::String| {
            let hub = hash_hub.clone();
            async move {
                let scope = hub.scope(&lua)?;
                let result = hub
                    .request(
                        scope,
                        EffectRequest::Crypto(
                            CryptoRequest::Sha256(bytes.as_bytes().to_vec()),
                            EffectOptions::bounded(Duration::from_secs(5)),
                        ),
                        cancellation(),
                    )
                    .await
                    .map_err(lua_error)?;
                let EffectResult::Crypto(bytes) = result else {
                    return Err(mlua::Error::runtime(
                        "crypto effect returned the wrong result",
                    ));
                };
                Ok(bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>())
            }
        })?,
    )?;
    crypto.set(
        "random_bytes",
        lua.create_async_function(move |lua, length: usize| {
            let hub = hub.clone();
            async move {
                let scope = hub.scope(&lua)?;
                let result = hub
                    .request(
                        scope,
                        EffectRequest::Crypto(
                            CryptoRequest::RandomBytes(length),
                            EffectOptions::bounded(Duration::from_secs(5)),
                        ),
                        cancellation(),
                    )
                    .await
                    .map_err(lua_error)?;
                let EffectResult::Crypto(bytes) = result else {
                    return Err(mlua::Error::runtime(
                        "crypto effect returned the wrong result",
                    ));
                };
                lua.create_string(&bytes)
            }
        })?,
    )?;
    pi.set("crypto", crypto)
}
