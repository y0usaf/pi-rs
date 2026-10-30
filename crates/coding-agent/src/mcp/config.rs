//! `core/mcp-servers.ts` and `extensions/mcp/config.ts`: a server entry of the `mcpServers`
//! shape, its validation and the messages for a bad one, and where the entries come from: the
//! global `mcp.json` and, in a trusted project, `<project>/.pi/mcp.json`, whose entries replace
//! global ones by name and may override only `enabled`, `exposure` and `toolExposure`.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::config::CONFIG_DIR_NAME;
use crate::paths::join;

/// MCP resource tools, which reach every server with resources.
pub const LIST_MCP_RESOURCES_TOOL: &str = "list_mcp_resources";
pub const LIST_MCP_RESOURCE_TEMPLATES_TOOL: &str = "list_mcp_resource_templates";
pub const READ_MCP_RESOURCE_TOOL: &str = "read_mcp_resource";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exposure {
    Codemode,
    Deferred,
    Direct,
    Hidden,
}

impl Exposure {
    pub const ALL: [Exposure; 4] = [
        Exposure::Codemode,
        Exposure::Deferred,
        Exposure::Direct,
        Exposure::Hidden,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Codemode => "codemode",
            Self::Deferred => "deferred",
            Self::Direct => "direct",
            Self::Hidden => "hidden",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "codemode" | "codemode-deferred" => Some(Self::Codemode),
            "deferred" => Some(Self::Deferred),
            "direct" => Some(Self::Direct),
            "hidden" => Some(Self::Hidden),
            _ => None,
        }
    }

    fn list() -> String {
        Self::ALL
            .iter()
            .map(|exposure| format!("\"{}\"", exposure.name()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// What every server entry shares.
#[derive(Debug, Clone)]
pub struct Base {
    pub exposure: Exposure,
    pub enabled: bool,
    pub description: Option<String>,
    pub tool_exposure: Vec<(String, Exposure)>,
    /// Per-request timeout in seconds.
    pub timeout: Option<f64>,
}

impl Default for Base {
    fn default() -> Self {
        Self {
            exposure: Exposure::Codemode,
            enabled: true,
            description: None,
            tool_exposure: Vec::new(),
            timeout: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ServerConfig {
    Stdio {
        base: Base,
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: Option<String>,
    },
    Http {
        base: Base,
        url: String,
        headers: BTreeMap<String, String>,
        /// `auth.provider`, the pi provider whose token the server wants.
        auth_provider: Option<String>,
        /// The validated `oauth` settings, which this port reads nothing of yet.
        oauth: Option<Value>,
    },
}

impl ServerConfig {
    pub fn base(&self) -> &Base {
        match self {
            Self::Stdio { base, .. } | Self::Http { base, .. } => base,
        }
    }

    /// `usesOAuth`: an HTTP server authenticates with OAuth unless a header or `auth` covers it.
    pub fn uses_oauth(&self) -> bool {
        match self {
            Self::Http {
                headers,
                auth_provider,
                ..
            } => {
                auth_provider.is_none()
                    && !headers
                        .keys()
                        .any(|header| header.eq_ignore_ascii_case("authorization"))
            }
            Self::Stdio { .. } => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Project,
}

/// One configured server.
#[derive(Debug, Clone)]
pub struct ServerEntry {
    pub name: String,
    pub config: ServerConfig,
    /// The `mcp.json` that defined the entry.
    pub source: String,
    pub scope: Scope,
    /// The project `mcp.json` that overrides a global server, where `/mcp` would save to.
    pub override_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Loaded {
    pub entries: Vec<ServerEntry>,
    pub errors: Vec<String>,
    pub project_config: Option<String>,
}

/// Namespace of a server's tools: `mcp__<server>` with `-` replaced by `_`.
pub fn namespace(server: &str) -> String {
    format!("mcp__{}", server.replace('-', "_"))
}

/// Exposure of one tool of a server: its `toolExposure` entry, else the server's `exposure`. An
/// exact name wins over patterns, and among patterns the first in the object wins.
pub fn tool_exposure(config: &ServerConfig, tool: &str) -> Exposure {
    let base = config.base();
    if let Some((_, exposure)) = base.tool_exposure.iter().find(|(name, _)| name == tool) {
        return *exposure;
    }
    base.tool_exposure
        .iter()
        .find(|(name, _)| name.contains('*') && glob_match(tool, name))
        .map(|(_, exposure)| *exposure)
        .unwrap_or(base.exposure)
}

/// A `*` pattern from `toolExposure`, `--tools` and `--exclude-tools`, matched against a name.
fn glob_match(name: &str, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut rest = name;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if index == 0 {
            let Some(after) = rest.strip_prefix(part) else {
                return false;
            };
            rest = after;
        } else if index == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            let Some(at) = rest.find(part) else {
                return false;
            };
            rest = &rest[at + part.len()..];
        }
    }
    true
}

fn is_record(value: &Value) -> bool {
    value.is_object()
}

fn is_string_record(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.values().all(Value::is_string))
}

fn string_field<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

/// A copy of the entry with exposure aliases (`codemode-deferred`) replaced by their current name.
fn resolve_exposure_aliases(value: &Map<String, Value>) -> Map<String, Value> {
    let mut resolved = value.clone();
    if let Some(Value::String(name)) = value.get("exposure")
        && let Some(exposure) = Exposure::parse(name)
    {
        resolved.insert("exposure".into(), Value::from(exposure.name()));
    }
    if let Some(tools) = value.get("toolExposure").and_then(Value::as_object) {
        let mapped: Map<String, Value> = tools
            .iter()
            .map(|(tool, entry)| {
                let resolved = match entry.as_str().and_then(Exposure::parse) {
                    Some(exposure) => Value::from(exposure.name()),
                    None => entry.clone(),
                };
                (tool.clone(), resolved)
            })
            .collect();
        resolved.insert("toolExposure".into(), Value::Object(mapped));
    }
    resolved
}

fn validate_oauth(value: Option<&Value>) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    let Some(object) = value.as_object() else {
        return Err("oauth must be an object".into());
    };
    for key in [
        "clientId",
        "clientSecret",
        "scope",
        "clientName",
        "clientRegistration",
        "callbackUrl",
        "authServerMetadataUrl",
    ] {
        if object.get(key).is_some_and(|field| !field.is_string()) {
            return Err(format!("oauth.{key} must be a string"));
        }
    }
    if let Some(port) = object.get("callbackPort")
        && !port
            .as_u64()
            .is_some_and(|port| (1..=65535).contains(&port))
    {
        return Err("oauth.callbackPort must be a port number".into());
    }
    if let Some("dcr" | "cimd") = string_field(object, "clientRegistration") {
    } else if object.contains_key("clientRegistration") {
        return Err("oauth.clientRegistration must be \"dcr\" or \"cimd\"".into());
    }
    Ok(())
}

fn parse_base(value: &Map<String, Value>, name: &str) -> Result<Base, String> {
    let exposures = Exposure::list();
    let mut base = Base::default();
    if let Some(exposure) = value.get("exposure") {
        match exposure.as_str().and_then(Exposure::parse) {
            Some(parsed) => base.exposure = parsed,
            None => {
                return Err(format!(
                    "server \"{name}\": exposure must be one of {exposures}"
                ));
            }
        }
    }
    if let Some(tools) = value.get("toolExposure") {
        let Some(object) = tools.as_object() else {
            return Err(format!(
                "server \"{name}\": toolExposure must map tool names to exposures"
            ));
        };
        for (tool, exposure) in object {
            match exposure.as_str().and_then(Exposure::parse) {
                Some(parsed) => base.tool_exposure.push((tool.clone(), parsed)),
                None => {
                    return Err(format!(
                        "server \"{name}\": toolExposure \"{tool}\" must be one of {exposures}"
                    ));
                }
            }
        }
    }
    if let Some(enabled) = value.get("enabled") {
        match enabled.as_bool() {
            Some(enabled) => base.enabled = enabled,
            None => return Err(format!("server \"{name}\": enabled must be a boolean")),
        }
    }
    if let Some(description) = value.get("description") {
        match description.as_str() {
            Some(description) => base.description = Some(description.to_owned()),
            None => return Err(format!("server \"{name}\": description must be a string")),
        }
    }
    if let Some(timeout) = value.get("timeout") {
        match timeout.as_f64() {
            Some(seconds) if seconds > 0.0 => base.timeout = Some(seconds),
            _ => {
                return Err(format!(
                    "server \"{name}\": timeout must be a positive number of seconds"
                ));
            }
        }
    }
    Ok(base)
}

/// Validate one entry of the `mcpServers` shape: the config, or the message for it.
pub fn validate(name: &str, raw: &Value) -> Result<ServerConfig, String> {
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        || name.is_empty()
    {
        return Err(format!(
            "invalid server name \"{name}\" (use letters, digits, \"_\" and \"-\")"
        ));
    }
    let Some(raw_object) = raw.as_object() else {
        return Err(format!("server \"{name}\" must be an object"));
    };
    let value = resolve_exposure_aliases(raw_object);
    let base = parse_base(&value, name)?;
    if string_field(&value, "type") == Some("sse") {
        return Err(format!(
            "server \"{name}\": legacy SSE transport is not supported; use the streamable HTTP URL"
        ));
    }
    let kind = string_field(&value, "type");
    if let Some(url) = string_field(&value, "url")
        && matches!(kind, None | Some("http" | "streamable-http"))
    {
        if !is_http_url(url) {
            return Err(format!(
                "server \"{name}\": url must be an http or https URL"
            ));
        }
        let headers = match value.get("headers") {
            None => BTreeMap::new(),
            Some(headers) if is_string_record(headers) => headers
                .as_object()
                .map(|object| {
                    object
                        .iter()
                        .map(|(key, value)| {
                            (key.clone(), value.as_str().unwrap_or_default().to_owned())
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Some(_) => {
                return Err(format!(
                    "server \"{name}\": headers must map names to strings"
                ));
            }
        };
        validate_oauth(value.get("oauth"))
            .map_err(|error| format!("server \"{name}\": {error}"))?;
        let auth_provider = match value.get("auth") {
            None => None,
            Some(auth) => {
                let provider = auth
                    .as_object()
                    .and_then(|object| string_field(object, "provider"))
                    .filter(|provider| !provider.is_empty());
                match provider {
                    Some(provider) => {
                        if !is_secure_or_loopback(url) {
                            return Err(format!(
                                "server \"{name}\": auth requires an https URL, or http on localhost, 127.0.0.1, or [::1]"
                            ));
                        }
                        Some(provider.to_owned())
                    }
                    None => {
                        return Err(format!(
                            "server \"{name}\": auth.provider must be a provider name"
                        ));
                    }
                }
            }
        };
        return Ok(ServerConfig::Http {
            base,
            url: url.to_owned(),
            headers,
            auth_provider,
            oauth: value.get("oauth").cloned(),
        });
    }
    if let Some(command) = string_field(&value, "command")
        && matches!(kind, None | Some("stdio"))
    {
        let args = match value.get("args") {
            None => Vec::new(),
            Some(Value::Array(items)) if items.iter().all(Value::is_string) => items
                .iter()
                .map(|item| item.as_str().unwrap_or_default().to_owned())
                .collect(),
            Some(_) => {
                return Err(format!(
                    "server \"{name}\": args must be an array of strings"
                ));
            }
        };
        let env = match value.get("env") {
            None => BTreeMap::new(),
            Some(env) if is_string_record(env) => env
                .as_object()
                .map(|object| {
                    object
                        .iter()
                        .map(|(key, value)| {
                            (key.clone(), value.as_str().unwrap_or_default().to_owned())
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Some(_) => {
                return Err(format!("server \"{name}\": env must map names to strings"));
            }
        };
        let cwd = match value.get("cwd") {
            None => None,
            Some(cwd) => match cwd.as_str() {
                Some(cwd) => Some(cwd.to_owned()),
                None => return Err(format!("server \"{name}\": cwd must be a string")),
            },
        };
        return Ok(ServerConfig::Stdio {
            base,
            command: command.to_owned(),
            args,
            env,
            cwd,
        });
    }
    Err(format!(
        "server \"{name}\" needs either \"command\" (stdio) or \"url\" (streamable HTTP)"
    ))
}

fn is_http_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"));
    rest.is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
}

fn is_secure_or_loopback(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or_default();
    let host = authority.split(':').next().unwrap_or_default();
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

fn read_object(path: &str) -> Result<Option<Map<String, Value>>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{path}: {error}")),
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let parsed: Value = serde_json::from_str(text).map_err(|error| format!("{path}: {error}"))?;
    if !is_record(&parsed)
        || parsed
            .get("mcpServers")
            .is_some_and(|servers| !is_record(servers))
    {
        return Err(format!(
            "{path}: expected an object with an \"mcpServers\" object"
        ));
    }
    Ok(parsed.as_object().cloned())
}

const OVERRIDE_KEYS: [&str; 3] = ["enabled", "exposure", "toolExposure"];

/// Whether an entry overrides a server defined elsewhere instead of defining one.
fn is_override(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        !object.contains_key("command")
            && !object.contains_key("url")
            && !object.contains_key("type")
    })
}

fn read_config(path: &str, scope: Scope, entries: &mut Vec<ServerEntry>, errors: &mut Vec<String>) {
    let parsed = match read_object(path) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => return,
        Err(error) => {
            errors.push(error);
            return;
        }
    };
    let Some(Value::Object(servers)) = parsed.get("mcpServers") else {
        return;
    };
    for (name, value) in servers {
        if scope == Scope::Project && is_override(value) {
            let base = entries.iter().position(|entry| entry.name == *name);
            let extra: Vec<&String> = value
                .as_object()
                .map(|object| {
                    object
                        .keys()
                        .filter(|key| !OVERRIDE_KEYS.contains(&key.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            let Some(base) = base else {
                errors.push(format!(
                    "{path}: server \"{name}\" needs \"command\" or \"url\", or a global server to override"
                ));
                continue;
            };
            if !extra.is_empty() {
                errors.push(format!(
                    "{path}: server \"{name}\": an override can only set {}",
                    OVERRIDE_KEYS.join(", ")
                ));
                continue;
            }
            let merged = merge_override(&entries[base], value);
            match validate(name, &merged) {
                Ok(config) => {
                    entries[base] = ServerEntry {
                        name: name.clone(),
                        config,
                        source: path.to_owned(),
                        scope: Scope::Project,
                        override_path: Some(path.to_owned()),
                    };
                }
                Err(error) => errors.push(format!("{path}: {error}")),
            }
            continue;
        }
        let config = match validate(name, value) {
            Ok(config) => config,
            Err(error) => {
                errors.push(format!("{path}: {error}"));
                continue;
            }
        };
        if let Some(clash) = entries
            .iter()
            .find(|entry| entry.name != *name && namespace(&entry.name) == namespace(name))
        {
            errors.push(format!(
                "{path}: server \"{name}\" conflicts with \"{}\"",
                clash.name
            ));
            continue;
        }
        if scope == Scope::Project
            && matches!(
                &config,
                ServerConfig::Http {
                    auth_provider: Some(_),
                    ..
                }
            )
        {
            errors.push(format!(
                "{path}: server \"{name}\": auth is only allowed in the global mcp.json"
            ));
            continue;
        }
        entries.push(ServerEntry {
            name: name.clone(),
            config,
            source: path.to_owned(),
            scope,
            override_path: None,
        });
    }
}

/// The global entry's JSON with the project override's keys on top.
fn merge_override(entry: &ServerEntry, value: &Value) -> Value {
    let mut object = entry_json(&entry.config);
    if let Some(overrides) = value.as_object() {
        for (key, value) in overrides {
            object.insert(key.clone(), value.clone());
        }
    }
    Value::Object(object)
}

/// A config as the JSON it was read from, for an override to change.
fn entry_json(config: &ServerConfig) -> Map<String, Value> {
    let mut object = Map::new();
    let base = config.base();
    if let Some(description) = &base.description {
        object.insert("description".into(), Value::from(description.clone()));
    }
    if let Some(timeout) = base.timeout {
        object.insert("timeout".into(), Value::from(timeout));
    }
    if !base.enabled {
        object.insert("enabled".into(), Value::from(false));
    }
    if base.exposure != Exposure::Codemode {
        object.insert("exposure".into(), Value::from(base.exposure.name()));
    }
    if !base.tool_exposure.is_empty() {
        let tools: Map<String, Value> = base
            .tool_exposure
            .iter()
            .map(|(tool, exposure)| (tool.clone(), Value::from(exposure.name())))
            .collect();
        object.insert("toolExposure".into(), Value::Object(tools));
    }
    match config {
        ServerConfig::Stdio {
            command,
            args,
            env,
            cwd,
            ..
        } => {
            object.insert("command".into(), Value::from(command.clone()));
            if !args.is_empty() {
                object.insert(
                    "args".into(),
                    Value::Array(args.iter().map(|arg| Value::from(arg.clone())).collect()),
                );
            }
            if !env.is_empty() {
                let env: Map<String, Value> = env
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::from(value.clone())))
                    .collect();
                object.insert("env".into(), Value::Object(env));
            }
            if let Some(cwd) = cwd {
                object.insert("cwd".into(), Value::from(cwd.clone()));
            }
        }
        ServerConfig::Http {
            url,
            headers,
            auth_provider,
            oauth,
            ..
        } => {
            object.insert("url".into(), Value::from(url.clone()));
            if !headers.is_empty() {
                let headers: Map<String, Value> = headers
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::from(value.clone())))
                    .collect();
                object.insert("headers".into(), Value::Object(headers));
            }
            if let Some(provider) = auth_provider {
                object.insert("auth".into(), serde_json::json!({ "provider": provider }));
            }
            if let Some(oauth) = oauth {
                object.insert("oauth".into(), oauth.clone());
            }
        }
    }
    object
}

/// Load the global `mcp.json` and, in a trusted project, the project's. Disabled servers are
/// included with `enabled: false`, so they can be enabled again.
pub fn load(agent_dir: &str, cwd: &str, project_trusted: bool) -> Loaded {
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    read_config(
        &join(agent_dir, "mcp.json"),
        Scope::Global,
        &mut entries,
        &mut errors,
    );
    let project = project_trusted
        .then(|| join(cwd, &format!("{CONFIG_DIR_NAME}/mcp.json")))
        .filter(|path| std::path::Path::new(path).exists());
    if let Some(path) = &project {
        read_config(path, Scope::Project, &mut entries, &mut errors);
    }
    Loaded {
        entries,
        errors,
        project_config: project,
    }
}
