//! `extensions/mcp/runtime.ts`: one connection per server, its transport, and the tools it
//! offers. Each connection owns a thread: it connects, lists the tools, then answers calls until
//! the session ends.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pi_durable::harness::types::ToolRegistration;
use pi_mcp::StreamableHttpTransport;
use pi_mcp::client::McpClient;
use pi_mcp::protocol::{CallToolResult, Root, Tool};
use pi_mcp::stdio::{StdioTransport, StdioTransportOptions};
use serde_json::Value;

use crate::config::VERSION;
use crate::mcp::config::{self, Exposure, ServerConfig, ServerEntry};
use crate::mcp::tool;

const DEFAULT_TIMEOUT_SECONDS: f64 = 60.0;
const STDERR_TAIL_CHARS: usize = 2_000;
/// How long the first prompt waits for the servers that declare tools to the model.
const STARTUP_WAIT: Duration = Duration::from_secs(10);
const MAX_SERVER_DESCRIPTION_CHARS: usize = 250;
const MAX_SERVERS_SECTION_CHARS: usize = 4_096;

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Connecting,
    Connected,
    /// The failure's first line for a list, and the whole message a call reports.
    Failed {
        detail: String,
        message: String,
    },
}

impl State {
    /// Short state for a list. `with_error` appends the first line of a failure.
    fn describe(&self, with_error: bool) -> String {
        match self {
            Self::Connecting => "connecting…".into(),
            Self::Connected => "connected".into(),
            Self::Failed { detail, .. } => {
                if with_error {
                    format!(
                        "failed: {}",
                        detail.lines().next().unwrap_or("unknown error")
                    )
                } else {
                    "failed".into()
                }
            }
        }
    }
}

enum Command {
    Call {
        tool: String,
        arguments: Option<Value>,
        timeout: Duration,
        reply: Sender<Result<CallToolResult, String>>,
    },
    Shutdown,
}

pub struct Connection {
    pub name: String,
    pub state: Mutex<State>,
    pub tools: Mutex<Vec<Tool>>,
    /// Server instructions from `initialize`, describing its tools as a group.
    pub instructions: Mutex<Option<String>>,
    timeout: Duration,
    commands: Mutex<Sender<Command>>,
}

impl Connection {
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn state(&self) -> State {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or(State::Connecting)
    }

    pub fn tools(&self) -> Vec<Tool> {
        self.tools
            .lock()
            .map(|tools| tools.clone())
            .unwrap_or_default()
    }

    pub fn call_tool(
        &self,
        tool: &str,
        arguments: Option<Value>,
        timeout: Duration,
    ) -> Result<CallToolResult, String> {
        if let State::Failed { message, .. } = self.state() {
            return Err(message);
        }
        let (reply, answer) = mpsc::channel();
        let sent = self
            .commands
            .lock()
            .map_err(|_| format!("MCP server \"{}\" is shut down", self.name))?
            .send(Command::Call {
                tool: tool.to_owned(),
                arguments,
                timeout,
                reply,
            });
        if sent.is_err() {
            return Err(format!("MCP server \"{}\" is shut down", self.name));
        }
        match answer.recv_timeout(timeout + Duration::from_secs(5)) {
            Ok(result) => result,
            Err(_) => Err("MCP request timed out".into()),
        }
    }

    fn set_failed(&self, detail: String, message: String) {
        if let Ok(mut state) = self.state.lock() {
            *state = State::Failed { detail, message };
        }
    }

    fn set_connected(&self, tools: Vec<Tool>, instructions: Option<String>) {
        if let Ok(mut error) = self.instructions.lock() {
            *error = instructions;
        }
        if let Ok(mut current) = self.tools.lock() {
            *current = tools;
        }
        if let Ok(mut state) = self.state.lock() {
            *state = State::Connected;
        }
    }
}

/// `~ and ~/` name the home directory, like in a shell.
fn expand_home(value: &str) -> String {
    if value == "~" {
        return crate::paths::home_dir();
    }
    match value.strip_prefix("~/") {
        Some(rest) => crate::paths::join(&crate::paths::home_dir(), rest),
        None => value.to_owned(),
    }
}

fn client_for(entry: &ServerEntry, cwd: &str, timeout: Duration) -> Result<Box<McpClient>, String> {
    let name = &entry.name;
    let roots = vec![Root {
        uri: format!("file://{cwd}"),
        name: std::path::Path::new(cwd)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
    }];
    let transport: Box<dyn pi_mcp::Transport> = match &entry.config {
        ServerConfig::Stdio {
            command,
            args,
            env,
            cwd: server_cwd,
            ..
        } => {
            let mut resolved = Vec::new();
            for (key, value) in env {
                let value = crate::config_value::resolve_or_err(
                    value,
                    &format!("MCP server \"{name}\" env \"{key}\""),
                    None,
                )?;
                resolved.push((key.clone(), value));
            }
            let mut options = StdioTransportOptions::new(expand_home(command));
            options.args = args.iter().map(|arg| expand_home(arg)).collect();
            options.cwd =
                crate::paths::resolve(cwd, &expand_home(server_cwd.as_deref().unwrap_or(".")));
            options.env = resolved;
            Box::new(StdioTransport::new(options))
        }
        ServerConfig::Http {
            url,
            headers,
            auth_provider,
            ..
        } => {
            if let Some(provider) = auth_provider {
                return Err(format!(
                    "MCP server \"{name}\": auth.provider \"{provider}\" is not ported yet"
                ));
            }
            let resolved = crate::config_value::resolve_headers(
                Some(headers),
                &format!("MCP server \"{name}\""),
                None,
            )?
            .unwrap_or_default();
            Box::new(
                StreamableHttpTransport::new(url.clone(), resolved.into_iter().collect())
                    .with_timeout(Some(timeout)),
            )
        }
    };
    Ok(Box::new(McpClient::new(
        transport, "pi", VERSION, timeout, roots,
    )))
}

fn run(
    connection: Arc<Connection>,
    entry: ServerEntry,
    cwd: String,
    commands: Receiver<Command>,
    ready: Sender<()>,
) {
    let timeout = connection.timeout;
    let mut client = match client_for(&entry, &cwd, timeout) {
        Ok(client) => client,
        Err(error) => {
            let message = format!(
                "MCP server \"{}\" failed to connect: {error}",
                connection.name
            );
            connection.set_failed(error, message);
            let _ = ready.send(());
            return;
        }
    };
    if connect(&mut client, &connection).is_err() {
        let _ = ready.send(());
        return;
    }
    let _ = ready.send(());
    for command in commands {
        match command {
            Command::Call {
                tool,
                arguments,
                timeout,
                reply,
            } => {
                let result = client
                    .call_tool(&tool, arguments, timeout)
                    .map_err(|error| error.message());
                let _ = reply.send(result);
            }
            Command::Shutdown => break,
        }
    }
    client.close();
}

fn connect(client: &mut McpClient, connection: &Connection) -> Result<(), ()> {
    if let Err(error) = client.connect() {
        client.close();
        report_failure(client, connection, error.message());
        return Err(());
    }
    let tools = if client.has_tools() {
        match client.list_tools() {
            Ok(tools) => tools,
            Err(error) => {
                report_failure(client, connection, error.message());
                client.close();
                return Err(());
            }
        }
    } else {
        Vec::new()
    };
    let instructions = client
        .instructions()
        .map(str::to_owned)
        .filter(|text| !text.trim().is_empty());
    connection.set_connected(tools, instructions);
    Ok(())
}

fn report_failure(client: &McpClient, connection: &Connection, error: String) {
    let detail = match client_stderr(client) {
        Some(tail) => format!("{error}\n{tail}"),
        None => error,
    };
    let message = format!(
        "MCP server \"{}\" failed to connect: {detail}",
        connection.name
    );
    connection.set_failed(detail, message);
}

fn client_stderr(client: &McpClient) -> Option<String> {
    let tail = client.stderr_tail()?;
    let tail = tail.trim();
    if tail.is_empty() {
        return None;
    }
    let trimmed: String = {
        let chars: Vec<char> = tail.chars().collect();
        let start = chars.len().saturating_sub(STDERR_TAIL_CHARS);
        chars[start..].iter().collect()
    };
    Some(trimmed)
}

/// The servers `mcp.json` configures, connected.
pub struct Servers {
    entries: Vec<ServerEntry>,
    errors: Vec<String>,
    connections: Vec<Arc<Connection>>,
}

impl Servers {
    /// Connect the servers `mcp.json` configures, waiting up to ten seconds for the tools that are
    /// declared to the model. `no_mcp` (`--no-mcp`) reads no configuration at all.
    pub fn start(agent_dir: &str, cwd: &str, project_trusted: bool, no_mcp: bool) -> Self {
        if no_mcp {
            return Self {
                entries: Vec::new(),
                errors: Vec::new(),
                connections: Vec::new(),
            };
        }
        let loaded = config::load(agent_dir, cwd, project_trusted);
        let mut connections = Vec::new();
        let mut readies = Vec::new();
        for entry in loaded
            .entries
            .iter()
            .filter(|entry| entry.config.base().enabled)
        {
            let (connection, ready) = start_connection(entry, cwd);
            connections.push(connection);
            readies.push(ready);
        }
        let deadline = Instant::now() + STARTUP_WAIT;
        for ready in readies {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match ready.recv_timeout(remaining) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => {}
                Err(RecvTimeoutError::Timeout) => break,
            }
        }
        Self {
            entries: loaded.entries,
            errors: loaded.errors,
            connections,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.connections.is_empty() && self.errors.is_empty()
    }

    /// Whether some server's tools are only reachable through codemode or `tool_search`, which this
    /// port has neither of.
    pub fn unreachable_warning(&self) -> Option<String> {
        let unreachable = self
            .entries
            .iter()
            .filter(|entry| entry.config.base().enabled)
            .any(|entry| {
                let exposures = std::iter::once(entry.config.base().exposure).chain(
                    entry
                        .config
                        .base()
                        .tool_exposure
                        .iter()
                        .map(|(_, exposure)| *exposure),
                );
                exposures
                    .into_iter()
                    .any(|exposure| matches!(exposure, Exposure::Codemode | Exposure::Deferred))
            });
        unreachable.then(|| {
            "MCP tools are only reachable from the codemode or tool_search tool, but neither is active; they cannot be called."
                .to_owned()
        })
    }

    /// Config errors and servers that failed to connect, one line each.
    pub fn problems(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .errors
            .iter()
            .map(|error| format!("config: {error}"))
            .collect();
        for connection in &self.connections {
            let state = connection.state();
            if matches!(state, State::Failed { .. }) {
                lines.push(format!("{}: {}", connection.name, state.describe(true)));
            }
        }
        lines
    }

    /// The servers' `direct` tools, as the agent's own tools.
    pub fn registrations(&self) -> Vec<ToolRegistration> {
        let mut owners: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut registrations = Vec::new();
        for connection in &self.connections {
            if connection.state() != State::Connected {
                continue;
            }
            let tools = connection.tools();
            let entry = self
                .entries
                .iter()
                .find(|entry| entry.name == connection.name);
            let Some(entry) = entry else {
                continue;
            };
            let mut plain: Vec<String> = Vec::with_capacity(tools.len());
            for tool in &tools {
                plain.push(tool::tool_name(&connection.name, &tool.name, &|_| false));
            }
            let mut assigned: std::collections::HashSet<String> = std::collections::HashSet::new();
            for tool in &tools {
                let owner = format!("{}\0{}", connection.name, tool.name);
                let name = tool::tool_name(&connection.name, &tool.name, &|candidate| {
                    owners
                        .get(candidate)
                        .is_some_and(|existing| existing != &owner)
                        || assigned.contains(candidate)
                        || plain.iter().filter(|plain| *plain == candidate).count() > 1
                });
                if config::tool_exposure(&entry.config, &tool.name) != Exposure::Direct {
                    continue;
                }
                owners.insert(name.clone(), owner);
                assigned.insert(name.clone());
                registrations.push(tool::registration(
                    Arc::clone(connection),
                    &connection.name,
                    tool,
                    name,
                    connection.timeout(),
                    false,
                ));
            }
        }
        registrations
    }

    /// The `mcp_servers` system prompt section: the enabled servers whose tools are not declared to
    /// the model, with a one-line summary each.
    pub fn section(&self) -> Option<String> {
        let mut listed: Vec<(&ServerEntry, String, String)> = Vec::new();
        for entry in &self.entries {
            if !entry.config.base().enabled {
                continue;
            }
            let base = entry.config.base();
            let reaches_codemode = std::iter::once(base.exposure)
                .chain(base.tool_exposure.iter().map(|(_, exposure)| *exposure))
                .any(|exposure| matches!(exposure, Exposure::Codemode | Exposure::Deferred));
            if !reaches_codemode {
                continue;
            }
            let instructions = self
                .connections
                .iter()
                .find(|connection| connection.name == entry.name)
                .and_then(|connection| {
                    connection
                        .instructions
                        .lock()
                        .ok()
                        .and_then(|instructions| instructions.clone())
                });
            listed.push((
                entry,
                config::namespace(&entry.name),
                server_summary(entry, instructions.as_deref()),
            ));
        }
        if listed.is_empty() {
            return None;
        }
        listed.sort_by(|a, b| a.0.name.cmp(&b.0.name));
        let intro = "MCP servers whose tools are not declared to you.";
        let heads: Vec<String> = listed
            .iter()
            .map(|(_, namespace, _)| format!("- {namespace} (codemode)"))
            .collect();
        let omitted = |count: usize| -> Vec<String> {
            if count == 0 {
                return Vec::new();
            }
            vec![format!(
                "- … {count} more server{}; find their tools with searchTools()",
                if count == 1 { "" } else { "s" }
            )]
        };
        let size = |kept: usize| -> usize {
            let mut lines = vec![intro.to_owned()];
            lines.extend(heads.iter().take(kept).cloned());
            lines.extend(omitted(listed.len() - kept));
            lines.join("\n").chars().count()
        };
        let mut kept = listed.len();
        while kept > 0 && size(kept) > MAX_SERVERS_SECTION_CHARS {
            kept -= 1;
        }
        let per_server = MAX_SERVERS_SECTION_CHARS
            .saturating_sub(size(kept))
            .checked_div(kept)
            .map(|budget| MAX_SERVER_DESCRIPTION_CHARS.min(budget.saturating_sub(2)))
            .unwrap_or(0);
        let mut lines = vec![intro.to_owned()];
        for (index, (_, _, summary)) in listed.iter().take(kept).enumerate() {
            let summary = if per_server == 0 {
                String::new()
            } else {
                truncate(summary, per_server)
            };
            if summary.is_empty() {
                lines.push(heads[index].clone());
            } else {
                lines.push(format!("{}: {summary}", heads[index]));
            }
        }
        lines.extend(omitted(listed.len() - kept));
        Some(lines.join("\n"))
    }
}

/// First line of the configured description, or of the server instructions once connected.
fn server_summary(entry: &ServerEntry, instructions: Option<&str>) -> String {
    let text = entry
        .config
        .base()
        .description
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .or(instructions)
        .unwrap_or_default();
    text.lines().next().unwrap_or_default().trim().to_owned()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    if max <= 1 {
        return String::new();
    }
    let mut cut: String = text.chars().take(max - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

fn start_connection(entry: &ServerEntry, cwd: &str) -> (Arc<Connection>, Receiver<()>) {
    let timeout = entry
        .config
        .base()
        .timeout
        .map(Duration::from_secs_f64)
        .unwrap_or_else(|| Duration::from_secs_f64(DEFAULT_TIMEOUT_SECONDS));
    let (commands, command_rx) = mpsc::channel();
    let connection = Arc::new(Connection {
        name: entry.name.clone(),
        state: Mutex::new(State::Connecting),
        tools: Mutex::new(Vec::new()),
        instructions: Mutex::new(None),
        timeout,
        commands: Mutex::new(commands),
    });
    let (ready, ready_rx) = mpsc::channel();
    let thread_connection = Arc::clone(&connection);
    let entry = entry.clone();
    let cwd = cwd.to_owned();
    std::thread::spawn(move || run(thread_connection, entry, cwd, command_rx, ready));
    (connection, ready_rx)
}

impl Drop for Servers {
    fn drop(&mut self) {
        for connection in &self.connections {
            if let Ok(commands) = connection.commands.lock() {
                let _ = commands.send(Command::Shutdown);
            }
        }
    }
}
