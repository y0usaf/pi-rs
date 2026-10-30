//! Opening a session, and switching to another one: the harness and root conversation a mode
//! talks to (`core/agent-session-runtime.ts`). The interactive loop and RPC mode both need this,
//! because a switch closes one session and opens the next.

use std::sync::Arc;

use pi_durable::ai::types::Model;
use pi_durable::cancel::CancelToken;
use pi_durable::entries::{assistant_entry, user_entry};
use pi_durable::errors::Result;
use pi_durable::harness::events::{AgentEvent, AgentEventStream};
use pi_durable::harness::harness::{Conversation, Harness};
use pi_durable::harness::types::{CompactionOverrides, ErrorReport};
use pi_durable::storage::memory::MemoryStorage;
use pi_durable::storage::sqlite::SqliteStorage;
use pi_durable::types::{EntryQuery, ScanOrder, Storage};

use crate::models::ModelRegistry;
use crate::print::{self, ToolSettings};

/// Where a session's events go: the interactive loop's channel, or RPC's stdout.
pub type Sink = Box<dyn FnMut(&[AgentEvent], &CancelToken) -> Result<()> + Send + 'static>;

/// What opening a session needs, beyond the path it lives at.
#[derive(Clone)]
pub struct Source {
    pub registry: Arc<ModelRegistry>,
    /// A model the catalog lacks (a --model id) is still callable.
    pub model: Model,
    pub tools: crate::loadout::ToolSelection,
    /// Tools other parts add beside the built-in ones (MCP's servers').
    pub extra_tools: Vec<pi_durable::harness::types::ToolRegistration>,
    pub prompt: Option<crate::system_prompt::PromptInput>,
    /// The tool environment without PI_SESSION_FILE, which depends on the session.
    pub tool_settings: ToolSettings,
    pub compaction: CompactionOverrides,
}

/// A session open in a mode: its harness, its root conversation and the subscription that feeds
/// the mode its events.
pub struct Open {
    pub harness: Harness,
    pub root: Conversation,
    pub stream: AgentEventStream,
    /// The sqlite file it lives in; None under --no-session.
    pub path: Option<String>,
    pub created: bool,
}

pub fn store(path: &str) -> std::result::Result<(Arc<dyn Storage>, bool), String> {
    let created = !std::path::Path::new(path).exists();
    std::fs::create_dir_all(crate::paths::dirname(path)).map_err(|e| e.to_string())?;
    let storage =
        SqliteStorage::open(path, &Default::default()).map_err(|e| format!("Error: {e}"))?;
    Ok((Arc::new(storage), created))
}

pub fn close(
    harness: &Harness,
    unstarted: Option<&str>,
    cancel: &CancelToken,
) -> std::result::Result<(), String> {
    let discard = unstarted.filter(|_| !has_conversation(harness, cancel));
    print::close(harness, cancel).map_err(|e| e.to_string())?;
    if let Some(path) = discard {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let _ = std::fs::remove_file(format!("{path}{suffix}"));
        }
    }
    Ok(())
}

fn has_conversation(harness: &Harness, cancel: &CancelToken) -> bool {
    let Ok(root) = print::root(harness, cancel) else {
        return true;
    };
    let query = EntryQuery {
        conversation_id: root.id(),
        min_entry_id: None,
        max_entry_id: None,
        order: Some(ScanOrder::Ascending),
    };
    let kinds = [user_entry(), assistant_entry()];
    let mut cursor = None;
    loop {
        let Ok(page) = root.entries(query.clone(), 256, cursor, cancel) else {
            return true;
        };
        if page
            .items
            .iter()
            .any(|record| kinds.iter().any(|kind| kind.kind() == record.kind))
        {
            return true;
        }
        match page.next {
            Some(next) => cursor = Some(next),
            None => return false,
        }
    }
}

impl Source {
    /// Open the session at `path`, or a fresh in-memory one for None, and feed its events to
    /// `sink`.
    pub fn open(
        &self,
        path: Option<&str>,
        report: ErrorReport,
        sink: Sink,
        cancel: &CancelToken,
    ) -> std::result::Result<Open, String> {
        let (storage, created): (Arc<dyn Storage>, bool) = match path {
            Some(path) => store(path)?,
            None => (Arc::new(MemoryStorage::new()), false),
        };
        let mut settings = self.tool_settings.clone();
        settings.env.retain(|(key, _)| key != "PI_SESSION_FILE");
        if let Some(path) = path {
            settings
                .env
                .push(("PI_SESSION_FILE".to_owned(), path.to_owned()));
        }
        let harness = print::open_harness_with(
            storage,
            print::providers(Arc::clone(&self.registry), &self.model),
            print::extension_with(
                &self.tools,
                self.prompt.clone(),
                settings,
                self.extra_tools.clone(),
            ),
            self.compaction.clone(),
            report,
        )
        .map_err(|e| e.to_string())?;
        let root = print::root(&harness, cancel).map_err(|e| e.to_string())?;
        let stream = root.events(cancel).map_err(|e| e.to_string())?;
        stream.start(sink).map_err(|e| e.to_string())?;
        // Picks up a run a killed process left behind.
        harness.resume().map_err(|e| e.to_string())?;
        Ok(Open {
            harness,
            root,
            stream,
            path: path.map(str::to_owned),
            created,
        })
    }
}

impl Open {
    /// Stop the event feed and close the harness, which settles its transaction and the file.
    pub fn shutdown(&mut self, cancel: &CancelToken) -> std::result::Result<(), String> {
        self.stream.stop();
        close(
            &self.harness,
            self.path.as_deref().filter(|_| self.created),
            cancel,
        )
    }

    /// Upstream's teardown before a session is replaced (`switchSession`, `newSession`, `fork`):
    /// settle the run in flight into the session it belongs to, close it, and open `target` in its
    /// place, feeding its events to `sink`.
    pub fn swap(
        &mut self,
        source: &Source,
        target: Option<&str>,
        report: ErrorReport,
        sink: Sink,
        cancel: &CancelToken,
    ) -> std::result::Result<(), String> {
        let _ = self.root.abort(false, cancel);
        let _ = self.root.wait_for_idle(cancel);
        self.shutdown(cancel)?;
        *self = source.open(target, report, sink, cancel)?;
        Ok(())
    }
}
