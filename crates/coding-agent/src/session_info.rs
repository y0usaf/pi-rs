//! What a session says about itself: its display name and, once forks exist, the session it was
//! forked from. Both live in one session-scope durable document, so a copy of the file carries
//! them. Each naming is also an entry on the branch, upstream's `session_info`, in the same commit.

use pi_durable::cancel::CancelToken;
use pi_durable::documents::{DocSemantics, DocToken};
use pi_durable::errors::Result;
use pi_durable::harness::harness::{Conversation, Harness};
use pi_durable::harness::util::{doc_definition, edit_session_doc};
use pi_durable::ids::{ConversationId, EntryId};
use pi_durable::session::types::Tx;
use pi_durable::types::EntryDraft;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Upstream's `session_info`: the entry kind of each naming, and the document holding the latest.
pub const KIND: &str = "pi.session-info";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionInfoState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The session file this one was forked from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session: Option<String>,
}

pub fn doc() -> Result<DocToken<SessionInfoState>> {
    doc_definition(
        KIND,
        1,
        DocSemantics::Session,
        SessionInfoState::default,
        Some(|_| true),
    )
}

/// `appendSessionInfo`'s name: line breaks collapsed to spaces, trimmed.
pub fn sanitize(name: &str) -> String {
    let mut out = String::new();
    let mut in_break = false;
    for ch in name.chars() {
        if ch == '\r' || ch == '\n' {
            if !in_break {
                out.push(' ');
            }
            in_break = true;
            continue;
        }
        in_break = false;
        out.push(ch);
    }
    out.trim().to_owned()
}

fn stored(harness: &Harness, cancel: &CancelToken) -> Result<Option<SessionInfoState>> {
    let token = doc()?;
    harness.session().snapshot_as(&token, None, None, cancel)
}

/// `getSessionName`: the stored name, or none when unset or empty.
pub fn name(harness: &Harness, cancel: &CancelToken) -> Option<String> {
    stored(harness, cancel)
        .ok()
        .flatten()
        .and_then(|state| state.name)
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// The session file this one was forked from, if it is a fork.
pub fn parent_session(harness: &Harness, cancel: &CancelToken) -> Option<String> {
    stored(harness, cancel)
        .ok()
        .flatten()
        .and_then(|s| s.parent_session)
}

fn write(
    conversation: &Conversation,
    cancel: &CancelToken,
    edit: impl FnOnce(&mut SessionInfoState),
) -> Result<()> {
    conversation.commit(
        |tx| {
            let token = doc()?;
            edit_session_doc(tx, &token, |state| edit(state))?;
            Ok(())
        },
        cancel,
    )
}

pub fn record(
    tx: &mut dyn Tx,
    conversation: ConversationId,
    name: &str,
    timestamp: &str,
) -> Result<EntryId> {
    let name = sanitize(name);
    let id = tx
        .append_entry(
            conversation,
            EntryDraft {
                kind: KIND.to_owned(),
                model: None,
                data: Some(json!({ "name": name, "timestamp": timestamp })),
                head: None,
                edits: None,
            },
        )?
        .id;
    let token = doc()?;
    edit_session_doc(tx, &token, |state| {
        state.name = (!name.is_empty()).then_some(name);
    })?;
    Ok(id)
}

/// Set the display name; an empty one clears it, as an empty `session_info` entry does.
pub fn set_name(conversation: &Conversation, cancel: &CancelToken, name: &str) -> Result<()> {
    let id = conversation.id();
    let timestamp = crate::sessions::iso8601(crate::sessions::export::now_ms());
    conversation.commit(|tx| record(tx, id, name, &timestamp).map(|_| ()), cancel)
}

/// Record the session file a fork came from (`parentSession`).
pub fn set_parent_session(
    conversation: &Conversation,
    cancel: &CancelToken,
    path: &str,
) -> Result<()> {
    write(conversation, cancel, |state| {
        state.parent_session = Some(path.to_owned());
    })
}
