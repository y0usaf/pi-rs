//! `core/` — mirrors the spec's `src/core/`. Modules land with their
//! workstreams (WS2.6: auth + models; WS3: sessions/settings; WS7:
//! resource loading).

pub mod auth_guidance;
pub mod defaults;
pub mod http_dispatcher;
pub mod model_resolver;

// `auth-storage.ts`, `model-registry.ts`, `resolve-config-value.ts`, and
// `settings-manager.ts` are ported in pi-rs-host (the substrate binds them
// to Lua as `pi.auth` / `pi.ai` / `pi.settings`); re-exported here so the
// coding-agent modules keep their spec-shaped paths.
pub use pi_rs_host::auth_storage;
pub use pi_rs_host::model_registry;
pub use pi_rs_host::resolve_config_value;
pub use pi_rs_host::settings_manager;
