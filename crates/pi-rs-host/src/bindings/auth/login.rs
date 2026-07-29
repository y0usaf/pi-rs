//! Subscription login: a Rust OAuth flow driven by Lua-authored UI callbacks.
//!
//! Rust owns only the wire mechanics the provider/auth subsystem already
//! implements: PKCE generation, the loopback callback server, authorization-code
//! exchange, and RFC 8628 device-code polling. Every user-visible step of a
//! login — the authorization URL, the device code, prompts, the login-method
//! selector, and progress — is an ordinary Lua function, so no login wording,
//! ordering, or presentation exists here.
//!
//! `login` returns the credential row instead of storing it: which store,
//! which location, and whether to keep the row at all stay `pi.auth.v1.store`
//! decisions in Lua.
//!
//! Concurrency shape: the flow runs as one Rust future while a second future
//! serves callback requests in arrival order, at most one Lua call in flight.
//! Serving is concurrent with the flow (a manual-code prompt does not stop the
//! callback server from settling) but never re-entrant, so a login consumes the
//! single bounded dispatch budget like any other host entry.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use mlua::{Function, Lua, Table, Value};
use pi_rs_ai_auth::{
    AuthError, AuthFuture, OAuthAuthInfo, OAuthCredentials, OAuthDeviceCodeInfo,
    OAuthLoginCallbacks, OAuthPrompt, OAuthSelectPrompt,
};
use tokio::sync::{mpsc, oneshot};

use crate::kernel::CancellationToken;

/// Wall-clock bound on one login when Lua names none. A device-code flow is
/// allowed a quarter hour by its own provider, so a shorter default would cut
/// off a legitimate login rather than bound a runaway one.
pub(super) const DEFAULT_LOGIN_TIMEOUT_MS: u64 = 15 * 60 * 1000;
/// Largest bound Lua may ask for.
pub(super) const MAX_LOGIN_TIMEOUT_MS: u64 = 60 * 60 * 1000;
/// Largest catalog-model list a login may enable after it succeeds.
pub(super) const MAX_LOGIN_MODELS: usize = 128;

/// One UI request a flow made, on its way to the Lua callback that serves it.
///
/// Reply channels carry successes only: a failing Lua callback ends the whole
/// login through the driver, so no error needs to travel back into the flow.
enum LoginRequest {
    Auth(OAuthAuthInfo),
    DeviceCode(OAuthDeviceCodeInfo),
    Progress(String),
    Prompt {
        prompt: OAuthPrompt,
        reply: oneshot::Sender<String>,
    },
    Select {
        prompt: OAuthSelectPrompt,
        reply: oneshot::Sender<Option<String>>,
    },
    ManualCode {
        reply: oneshot::Sender<String>,
    },
}

/// The Lua functions one login may call. `on_auth`, `on_device_code`,
/// `on_prompt`, and `on_select` are required because a flow that reaches one of
/// them cannot proceed without an answer; the rest are optional.
struct LoginCallbacks {
    on_auth: Function,
    on_device_code: Function,
    on_prompt: Function,
    on_select: Function,
    on_progress: Option<Function>,
    on_manual_code_input: Option<Function>,
}

impl LoginCallbacks {
    fn from_table(callbacks: &Table) -> mlua::Result<Self> {
        Ok(Self {
            on_auth: required(callbacks, "on_auth")?,
            on_device_code: required(callbacks, "on_device_code")?,
            on_prompt: required(callbacks, "on_prompt")?,
            on_select: required(callbacks, "on_select")?,
            on_progress: callbacks.get("on_progress")?,
            on_manual_code_input: callbacks.get("on_manual_code_input")?,
        })
    }
}

fn required(callbacks: &Table, name: &str) -> mlua::Result<Function> {
    callbacks
        .get::<Option<Function>>(name)?
        .ok_or_else(|| mlua::Error::runtime(format!("auth.v1 login requires callback {name}")))
}

/// The `OAuthLoginCallbacks` implementation the flow sees. It is `Send + Sync`
/// and holds no Lua value: every UI step becomes a queued request, which is what
/// lets a non-`Send` Lua VM drive a `Send` provider flow.
struct LoginBridge {
    requests: mpsc::UnboundedSender<LoginRequest>,
    cancellation: Option<CancellationToken>,
    manual_code_input: bool,
    model_ids: Vec<String>,
}

impl LoginBridge {
    /// Queue one reply-bearing request and wait for the Lua answer, racing the
    /// innermost dispatch cancellation so a cancelled dispatch never leaves a
    /// flow parked on a prompt.
    fn request<'a, T: Send + 'a>(
        &'a self,
        build: impl FnOnce(oneshot::Sender<T>) -> LoginRequest,
    ) -> AuthFuture<'a, T> {
        let (reply, answer) = oneshot::channel();
        let queued = self.requests.send(build(reply)).is_ok();
        let cancellation = self.cancellation.clone();
        Box::pin(async move {
            if !queued {
                return Err(AuthError::Cancelled);
            }
            match cancellation {
                Some(token) => tokio::select! {
                    biased;
                    () = token.cancelled() => Err(AuthError::Cancelled),
                    answer = answer => answer.map_err(|_| AuthError::Cancelled),
                },
                None => answer.await.map_err(|_| AuthError::Cancelled),
            }
        })
    }
}

impl OAuthLoginCallbacks for LoginBridge {
    fn on_auth(&self, info: OAuthAuthInfo) {
        let _ = self.requests.send(LoginRequest::Auth(info));
    }

    fn on_device_code(&self, info: OAuthDeviceCodeInfo) {
        let _ = self.requests.send(LoginRequest::DeviceCode(info));
    }

    fn on_progress(&self, message: &str) {
        let _ = self
            .requests
            .send(LoginRequest::Progress(message.to_owned()));
    }

    fn on_prompt(&self, prompt: OAuthPrompt) -> AuthFuture<'_, String> {
        self.request(|reply| LoginRequest::Prompt { prompt, reply })
    }

    fn on_select(&self, prompt: OAuthSelectPrompt) -> AuthFuture<'_, Option<String>> {
        self.request(|reply| LoginRequest::Select { prompt, reply })
    }

    fn on_manual_code_input(&self) -> Option<AuthFuture<'_, String>> {
        self.manual_code_input
            .then(|| self.request(|reply| LoginRequest::ManualCode { reply }))
    }

    fn provider_model_ids(&self, _provider: &str) -> Vec<String> {
        self.model_ids.clone()
    }

    fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }

    fn on_cancelled(&self) -> AuthFuture<'_, ()> {
        let cancellation = self.cancellation.clone();
        Box::pin(async move {
            match cancellation {
                Some(token) => {
                    token.cancelled().await;
                    Ok(())
                }
                None => std::future::pending().await,
            }
        })
    }
}

fn auth_info_table(lua: &Lua, info: &OAuthAuthInfo) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("url", info.url.as_str())?;
    table.set("instructions", info.instructions.clone())?;
    Ok(table)
}

fn device_code_table(lua: &Lua, info: &OAuthDeviceCodeInfo) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("user_code", info.user_code.as_str())?;
    table.set("verification_uri", info.verification_uri.as_str())?;
    table.set("interval_seconds", info.interval_seconds)?;
    table.set("expires_in_seconds", info.expires_in_seconds)?;
    Ok(table)
}

fn prompt_table(lua: &Lua, prompt: &OAuthPrompt) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("message", prompt.message.as_str())?;
    table.set("placeholder", prompt.placeholder.clone())?;
    table.set("allow_empty", prompt.allow_empty)?;
    Ok(table)
}

fn select_table(lua: &Lua, prompt: &OAuthSelectPrompt) -> mlua::Result<Table> {
    let options = lua.create_table_with_capacity(prompt.options.len(), 0)?;
    for option in &prompt.options {
        let row = lua.create_table()?;
        row.set("id", option.id.as_str())?;
        row.set("label", option.label.as_str())?;
        options.push(row)?;
    }
    let table = lua.create_table()?;
    table.set("message", prompt.message.as_str())?;
    table.set("options", options)?;
    Ok(table)
}

/// Serve one queued request from Lua. A raising callback ends the login: the
/// error travels out through the driver instead of being folded into a flow
/// diagnostic, so the package sees its own failure.
async fn serve(lua: &Lua, callbacks: &LoginCallbacks, request: LoginRequest) -> mlua::Result<()> {
    match request {
        LoginRequest::Auth(info) => {
            callbacks
                .on_auth
                .call_async::<()>(auth_info_table(lua, &info)?)
                .await
        }
        LoginRequest::DeviceCode(info) => {
            callbacks
                .on_device_code
                .call_async::<()>(device_code_table(lua, &info)?)
                .await
        }
        LoginRequest::Progress(message) => match &callbacks.on_progress {
            Some(callback) => callback.call_async::<()>(message).await,
            None => Ok(()),
        },
        LoginRequest::Prompt { prompt, reply } => {
            let answer = callbacks
                .on_prompt
                .call_async::<String>(prompt_table(lua, &prompt)?)
                .await?;
            super::check_secret("login prompt response", &answer)?;
            let _ = reply.send(answer);
            Ok(())
        }
        LoginRequest::Select { prompt, reply } => {
            let answer = callbacks
                .on_select
                .call_async::<Option<String>>(select_table(lua, &prompt)?)
                .await?;
            let _ = reply.send(answer);
            Ok(())
        }
        LoginRequest::ManualCode { reply } => {
            let Some(callback) = &callbacks.on_manual_code_input else {
                return Ok(());
            };
            let answer = callback.call_async::<String>(()).await?;
            super::check_secret("login manual code", &answer)?;
            let _ = reply.send(answer);
            Ok(())
        }
    }
}

fn timeout_ms(options: Option<&Table>) -> mlua::Result<u64> {
    let Some(options) = options else {
        return Ok(DEFAULT_LOGIN_TIMEOUT_MS);
    };
    let Some(requested) = options.get::<Option<u64>>("timeout_ms")? else {
        return Ok(DEFAULT_LOGIN_TIMEOUT_MS);
    };
    if requested == 0 || requested > MAX_LOGIN_TIMEOUT_MS {
        return Err(mlua::Error::runtime(format!(
            "auth.v1 login timeout_ms must be 1..={MAX_LOGIN_TIMEOUT_MS}"
        )));
    }
    Ok(requested)
}

/// Catalog model ids the flow may enable once login succeeds. The list is read
/// once, before the flow starts, because the provider interface asks for it
/// synchronously and a Lua call cannot be served from inside a poll.
async fn model_ids(provider: &str, options: Option<&Table>) -> mlua::Result<Vec<String>> {
    let Some(options) = options else {
        return Ok(Vec::new());
    };
    let Some(callback) = options.get::<Option<Function>>("model_ids")? else {
        return Ok(Vec::new());
    };
    let ids = callback
        .call_async::<Vec<String>>(provider.to_owned())
        .await?;
    if ids.len() > MAX_LOGIN_MODELS {
        return Err(mlua::Error::runtime(format!(
            "auth.v1 login model_ids returned more than {MAX_LOGIN_MODELS} ids"
        )));
    }
    Ok(ids)
}

/// Run one subscription login and return the credential row it produced.
pub(super) async fn run(
    lua: Lua,
    provider: String,
    callbacks: Table,
    options: Option<Table>,
) -> mlua::Result<Value> {
    let flow = pi_rs_ai_auth::get_oauth_provider(&provider)
        .ok_or_else(|| mlua::Error::runtime(format!("Unknown OAuth provider: {provider}")))?;
    let callbacks = LoginCallbacks::from_table(&callbacks)?;
    let timeout = timeout_ms(options.as_ref())?;
    let model_ids = model_ids(&provider, options.as_ref()).await?;
    let cancellation = crate::kernel_api::current_cancellation(&lua)?;

    let (requests, mut queue) = mpsc::unbounded_channel();
    let bridge = LoginBridge {
        requests,
        cancellation: cancellation.clone(),
        manual_code_input: callbacks.on_manual_code_input.is_some(),
        model_ids,
    };

    // One Lua call at a time, served in arrival order, concurrently with the
    // flow itself. Parking on channel close keeps this future alive for the
    // whole login: only an error escapes it.
    let served = async {
        while let Some(request) = queue.recv().await {
            serve(&lua, &callbacks, request).await?;
        }
        std::future::pending::<()>().await;
        Ok::<(), mlua::Error>(())
    };
    let cancelled: Pin<Box<dyn Future<Output = ()>>> = match cancellation {
        Some(token) => Box::pin(async move { token.cancelled().await }),
        None => Box::pin(std::future::pending()),
    };
    let login = flow.login(&bridge);

    tokio::select! {
        biased;
        result = served => match result {
            Ok(()) => Err(mlua::Error::runtime("auth.v1 login lost its callback queue")),
            Err(error) => Err(error),
        },
        // The cancel marker is what makes a cancelled login report as a
        // cancelled dispatch instead of an ordinary Lua failure.
        () = cancelled => Err(mlua::Error::runtime(format!(
            "{}: auth login",
            crate::error::CANCEL_MARKER
        ))),
        () = tokio::time::sleep(Duration::from_millis(timeout)) => Err(mlua::Error::runtime(
            format!("auth.v1 login exceeded {timeout}ms"),
        )),
        outcome = login => credentials(&lua, outcome.map_err(super::auth_error)?),
    }
}

/// The credential row a package hands straight to `store:set_oauth`: the three
/// required fields plus whatever provider-defined extra data the flow returned.
fn credentials(lua: &Lua, credentials: OAuthCredentials) -> mlua::Result<Value> {
    super::check_secret("oauth refresh token", &credentials.refresh)?;
    super::check_secret("oauth access token", &credentials.access)?;
    let json = serde_json::to_value(&credentials)
        .map_err(|error| mlua::Error::runtime(format!("invalid oauth credential: {error}")))?;
    crate::convert::json_to_lua(lua, &json)
}
