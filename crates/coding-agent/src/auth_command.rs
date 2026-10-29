//! cli/auth-command.ts, cli/auth-check.ts and cli/credential-print.ts: `pi-rs auth` prints a
//! provider's credential or checks that it is ready.

use crate::args;
use crate::auth::{Credential, OAUTH_MINIMUM_VALIDITY_MS};
use crate::model_resolver::{self, Reference, ReferenceRequest, ThinkingSuffix};
use crate::models::{ModelRegistry, credential_value};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

const APP_NAME: &str = "pi-rs";
const DEFAULT_BEARER_TOKEN_MIN_EXPIRY_MS: i64 = 30 * 60_000;

/// The subcommand.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Check,
    ApiKey,
    BearerToken,
}

/// The exit code pi-rs ends with, and what it writes to stdout.
pub enum Outcome {
    Text { stdout: String, code: u8 },
    Error(String),
}

impl Kind {
    /// `getAuthCommandName`.
    fn name(self) -> &'static str {
        match self {
            Kind::Check => "auth check",
            Kind::ApiKey => "auth print-api-key",
            Kind::BearerToken => "auth print-bearer-token",
        }
    }

    /// `AUTH_COMMAND_USAGE`.
    fn usage(self) -> String {
        match self {
            Kind::Check => format!(
                "{APP_NAME} auth check --provider <provider> [--json] [--credentials] [--no-refresh]"
            ),
            Kind::ApiKey => {
                format!("{APP_NAME} auth print-api-key --provider <provider> [--model <model>]")
            }
            Kind::BearerToken => format!(
                "{APP_NAME} auth print-bearer-token --provider <provider> [--model <model>] [--min-expiry <duration>]"
            ),
        }
    }
}

/// A parsed `auth` command line (`AuthCommand`).
struct Command {
    kind: Kind,
    args: Vec<String>,
    json: bool,
    credentials: bool,
    no_refresh: bool,
    min_expiry_ms: Option<i64>,
}

/// One `check` outcome (`AuthCheckResult`).
struct Check {
    status: &'static str,
    provider: String,
    reason: Option<&'static str>,
    auth_type: Option<&'static str>,
}

impl Check {
    fn ready(provider: String, auth_type: &'static str) -> Self {
        Self {
            status: "ready",
            provider,
            reason: None,
            auth_type: Some(auth_type),
        }
    }

    fn not_ready(provider: String, reason: &'static str) -> Self {
        Self {
            status: "not_ready",
            provider,
            reason: Some(reason),
            auth_type: None,
        }
    }

    fn invalid(provider: String) -> Self {
        Self {
            status: "invalid",
            provider,
            reason: Some("invalid_state"),
            auth_type: None,
        }
    }
}

const HELP: &str = "Usage:
  pi-rs auth print-api-key [--provider <provider>] [--model <model>]
  pi-rs auth print-bearer-token [--provider <provider>] [--model <model>] [--min-expiry <duration>]
  pi-rs auth check [--provider <provider>] [--model <model>] [--json] [--credentials] [--no-refresh]

Auth commands require at least one of --provider or --model. Checks refresh expired OAuth credentials by default; --no-refresh prevents this. --credentials emits the credential, or includes it in JSON output.";

/// `isAuthCommandHelp`.
fn is_help(args: &[String]) -> bool {
    args.first().map(String::as_str) == Some("auth")
        && (args.get(1).is_none()
            || args.get(1).map(String::as_str) == Some("help")
            || args.iter().any(|arg| arg == "--help" || arg == "-h"))
}

/// `parseAuthCommand`.
fn parse(args: &[String]) -> Result<Command, String> {
    let kind = match args.get(1).map(String::as_str) {
        Some("check") => Kind::Check,
        Some("print-api-key") => Kind::ApiKey,
        Some("print-bearer-token") => Kind::BearerToken,
        other => {
            return Err(format!(
                "Unknown auth command \"{}\". Use \"{APP_NAME} auth print-api-key\", \"{APP_NAME} auth print-bearer-token\", or \"{APP_NAME} auth check\".",
                other.unwrap_or("")
            ));
        }
    };
    let mut command = Command {
        kind,
        args: Vec::new(),
        json: false,
        credentials: false,
        no_refresh: false,
        min_expiry_ms: None,
    };
    let mut index = 2;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--min-expiry" {
            if kind != Kind::BearerToken {
                return Err("--min-expiry is only supported by print-bearer-token".to_owned());
            }
            index += 1;
            command.min_expiry_ms = Some(
                args.get(index)
                    .and_then(|value| duration_ms(value))
                    .ok_or_else(|| {
                        "--min-expiry must use a duration such as 30m or 1h".to_owned()
                    })?,
            );
        } else if arg == "--json" || arg == "--credentials" || arg == "--no-refresh" {
            if kind != Kind::Check {
                return Err(format!("{arg} is only supported by auth check"));
            }
            match arg.as_str() {
                "--json" => command.json = true,
                "--credentials" => command.credentials = true,
                _ => command.no_refresh = true,
            }
        } else {
            command.args.push(arg.clone());
        }
        index += 1;
    }
    Ok(command)
}

/// `--min-expiry`'s `^(\d+)(ms|s|m|h)$`.
fn duration_ms(text: &str) -> Option<i64> {
    let unit_at = text.find(|c: char| !c.is_ascii_digit())?;
    let (amount, unit) = text.split_at(unit_at);
    let factor: i64 = match unit.to_ascii_lowercase().as_str() {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        _ => return None,
    };
    amount.parse::<i64>().ok().map(|n| n.saturating_mul(factor))
}

/// `validateAuthCommandArgs`.
fn validate(parsed: &args::Args, kind: Kind) -> Result<(Option<String>, Option<String>), String> {
    let value = |text: Option<&String>| {
        text.map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    let provider = value(parsed.provider.as_ref());
    let model = value(parsed.model.as_ref());
    if parsed.api_key.is_some() || !parsed.messages.is_empty() || !parsed.file_args.is_empty() {
        return Err("Auth commands only accept --provider and --model".to_owned());
    }
    if provider.is_none() && model.is_none() {
        return Err(match kind {
            Kind::Check => "Auth checks require --provider <provider> or --model <model>",
            _ => "Credential printing requires --provider <provider> or --model <model>",
        }
        .to_owned());
    }
    Ok((provider, model))
}

/// `runAuthCommand`: runs an `auth` command line, or None when the arguments are something else.
pub fn run(args: &[String], agent_dir: &str) -> Option<Outcome> {
    if args.first().map(String::as_str) != Some("auth") {
        return None;
    }
    if is_help(args) {
        return Some(Outcome::Text {
            stdout: format!("{HELP}\n"),
            code: 0,
        });
    }
    let command = match parse(args) {
        Ok(command) => command,
        Err(message) => {
            return Some(Outcome::Error(message));
        }
    };
    let parsed = args::parse(&command.args);
    if let Some((option, _)) = parsed.unknown_flags.first() {
        eprintln!("Unknown option --{option} for \"{}\".", command.kind.name());
        eprintln!("Use \"{APP_NAME} --help\" or \"{}\".", command.kind.usage());
        return Some(Outcome::Text {
            stdout: String::new(),
            code: 1,
        });
    }
    if !parsed.diagnostics.is_empty() {
        let message = parsed
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        return Some(Outcome::Error(message));
    }
    let registry = ModelRegistry::load(agent_dir);
    let outcome = match validate(&parsed, command.kind) {
        Err(message) => Err(message),
        Ok((provider, model)) => match command.kind {
            Kind::Check => Ok(check(
                &registry,
                provider,
                model,
                command.json,
                command.credentials,
                command.no_refresh,
            )),
            kind => print_credential(&registry, kind, provider, model, command.min_expiry_ms).map(
                |credential| Outcome::Text {
                    stdout: format!("{credential}\n"),
                    code: 0,
                },
            ),
        },
    };
    Some(outcome.unwrap_or_else(Outcome::Error))
}

/// `resolveCredentialForPrint`.
fn print_credential(
    registry: &ModelRegistry,
    kind: Kind,
    provider: Option<String>,
    model: Option<String>,
    min_expiry_ms: Option<i64>,
) -> Result<String, String> {
    let stored: BTreeMap<String, &'static str> =
        registry.stored_credentials().into_iter().collect();
    let mut targets: Vec<String> = Vec::new();
    if let Some(id) = provider.clone() {
        if !registry.provider_exists(&id) {
            return Err(format!(
                "Unknown provider \"{id}\". Use --list-models to see available providers."
            ));
        }
        match model {
            Some(text) => {
                let resolved = model_resolver::resolve_cli_model(
                    Some(&id),
                    Some(&text),
                    None,
                    false,
                    &registry.models(),
                    &|p: &str| registry.has_configured_auth(p),
                );
                if let Some(error) = resolved.error {
                    return Err(error);
                }
                match resolved.model {
                    Some(found) => targets.push(found.provider),
                    None => {
                        return Err("Unable to resolve the requested provider/model".to_owned());
                    }
                }
            }
            None => targets.push(id),
        }
    } else {
        let Some(text) = model.as_deref() else {
            return Err("Unable to resolve an auth provider".to_owned());
        };
        let mut providers: Vec<&str> = Vec::new();
        let models = registry.models();
        for entry in models.iter() {
            if stored.contains_key(&entry.provider) && !providers.contains(&entry.provider.as_str())
            {
                providers.push(&entry.provider);
            }
        }
        for id in providers {
            let request = ReferenceRequest {
                text,
                provider: Some(id),
                thinking_suffix: ThinkingSuffix::Strict,
                fuzzy: true,
            };
            if let Reference::Resolved { model, .. } =
                model_resolver::resolve_model_reference(&request, &models, &|_| true)
            {
                targets.push(model.provider.clone());
            }
        }
        if targets.is_empty() {
            return Err(format!(
                "Model \"{text}\" not found. Use --list-models to see available models."
            ));
        }
    }

    let mut found: Vec<(String, String)> = Vec::new();
    for id in &targets {
        let stored_type = stored.get(id).copied();
        if kind == Kind::ApiKey && stored_type == Some("oauth") {
            continue;
        }
        if kind == Kind::BearerToken && stored_type != Some("oauth") {
            continue;
        }
        let validity = if kind == Kind::BearerToken {
            min_expiry_ms.unwrap_or(DEFAULT_BEARER_TOKEN_MIN_EXPIRY_MS)
        } else {
            OAUTH_MINIMUM_VALIDITY_MS
        };
        let auth = registry
            .provider_auth(id, validity)
            .map_err(|_| "Failed to resolve credential".to_owned())?;
        if let Some(value) = credential_value(&auth) {
            found.push((id.clone(), value));
        }
    }

    match found.len() {
        1 => Ok(found.remove(0).1),
        0 => {
            let first = targets.first().cloned().unwrap_or_default();
            let stored_type = stored.get(&first).copied();
            if provider.is_some() && kind == Kind::ApiKey && stored_type == Some("oauth") {
                return Err(format!(
                    "Provider \"{first}\" is configured with OAuth, not an API key"
                ));
            }
            if provider.is_some() && kind == Kind::BearerToken && stored_type != Some("oauth") {
                return Err(format!(
                    "Provider \"{first}\" is not configured with an OAuth bearer token"
                ));
            }
            Err(format!(
                "No usable {} is configured",
                if kind == Kind::ApiKey {
                    "API key"
                } else {
                    "OAuth bearer token"
                }
            ))
        }
        _ => Err(format!(
            "Multiple configured providers matched ({}). Specify --provider.",
            found
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// `runAuthCommand`'s `check` branch: `checkProviderAuth` and `getProviderCredential`.
fn check(
    registry: &ModelRegistry,
    provider: Option<String>,
    model: Option<String>,
    json: bool,
    credentials: bool,
    no_refresh: bool,
) -> Outcome {
    let requested = provider
        .clone()
        .or_else(|| model.clone())
        .unwrap_or_default();
    let mut provider = provider;
    if let Some(text) = model.as_deref() {
        let resolved = model_resolver::resolve_cli_model(
            provider.as_deref(),
            Some(text),
            None,
            false,
            &registry.models(),
            &|p: &str| registry.has_configured_auth(p),
        );
        match resolved.model.filter(|_| resolved.error.is_none()) {
            Some(found) => provider = Some(found.provider),
            None => return checked(Check::invalid(requested), None, json),
        }
    }
    let Some(id) = provider else {
        return checked(Check::invalid(requested), None, json);
    };
    let mut result = if registry.error().is_some() {
        Check::invalid(id)
    } else if !registry.provider_exists(&id) {
        Check::not_ready(id, "provider_not_found")
    } else {
        match registry.check_auth(&id) {
            None => Check::not_ready(id, "credentials_not_configured"),
            Some(auth_type) if no_refresh => Check::ready(id, auth_type),
            Some(auth_type) => match registry.provider_auth(&id, OAUTH_MINIMUM_VALIDITY_MS) {
                Ok(auth) if credential_value(&auth).is_some() => Check::ready(id, auth_type),
                Ok(_) => Check::not_ready(id, "credentials_not_configured"),
                Err(_) => Check::invalid(id),
            },
        }
    };
    let mut credential = None;
    if credentials && result.status == "ready" {
        match provider_credential(registry, &result.provider, no_refresh) {
            Ok(Some(value)) => credential = Some(value),
            Ok(None) => result = Check::not_ready(result.provider, "credential_not_available"),
            Err(_) => result = Check::invalid(result.provider),
        }
    }
    checked(result, credential, json)
}

/// `getProviderCredential`.
fn provider_credential(
    registry: &ModelRegistry,
    provider: &str,
    no_refresh: bool,
) -> Result<Option<String>, ()> {
    if no_refresh && let Some(Credential::OAuth(stored)) = registry.credential(provider) {
        return Ok(stored
            .get("access")
            .and_then(Value::as_str)
            .map(str::to_owned));
    }
    registry
        .provider_auth(provider, OAUTH_MINIMUM_VALIDITY_MS)
        .map(|auth| credential_value(&auth))
        .map_err(|_| ())
}

/// The status line (or the credential) and the exit code: ready 0, not_ready 1, invalid 2.
fn checked(result: Check, credential: Option<String>, json: bool) -> Outcome {
    let stdout = if json {
        let mut object = Map::new();
        object.insert("status".to_owned(), Value::String(result.status.to_owned()));
        object.insert("provider".to_owned(), Value::String(result.provider));
        if let Some(reason) = result.reason {
            object.insert("reason".to_owned(), Value::String(reason.to_owned()));
        }
        if let Some(auth_type) = result.auth_type {
            object.insert("authType".to_owned(), Value::String(auth_type.to_owned()));
        }
        if let Some(credential) = credential {
            object.insert("credentials".to_owned(), Value::String(credential));
        }
        format!("{}\n", Value::Object(object))
    } else {
        format!("{}\n", credential.as_deref().unwrap_or(result.status))
    };
    Outcome::Text {
        stdout,
        code: match result.status {
            "ready" => 0,
            "not_ready" => 1,
            _ => 2,
        },
    }
}
