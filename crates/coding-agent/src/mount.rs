use std::sync::{Arc, OnceLock};

use pi_durable::ai::models::Models;
use pi_durable::ai::registry::Providers;
use pi_durable::ai::types::Model;
use pi_durable::harness::types::Extension;

pub struct Mounted {
    pub models: Arc<dyn Models + Send + Sync>,
    pub extensions: Vec<Extension>,
}

pub type Open = Box<dyn Fn(Providers) -> Mounted + Send + Sync>;

pub struct Mount {
    pub catalog: Vec<Model>,
    pub open: Open,
}

static MOUNT: OnceLock<Mount> = OnceLock::new();

pub fn set(mount: Mount) -> Result<(), String> {
    MOUNT
        .set(mount)
        .map_err(|_| "a mount is already set".to_owned())
}

pub fn catalog() -> &'static [Model] {
    MOUNT.get().map_or(&[], |mount| &mount.catalog)
}

pub fn mounts(provider: &str) -> bool {
    catalog().iter().any(|model| model.provider == provider)
}

pub fn open(providers: Providers) -> Mounted {
    match MOUNT.get() {
        Some(mount) => (mount.open)(providers),
        None => Mounted {
            models: Arc::new(providers),
            extensions: Vec::new(),
        },
    }
}
