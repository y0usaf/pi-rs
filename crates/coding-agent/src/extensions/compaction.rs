use pi_durable::ai::types::Usage;
use pi_durable::cancel::CancelToken;
use pi_durable::entries::compaction_entry;
use pi_durable::harness::compaction::{
    COMPACTION_HOOK, CompactionDecision, CompactionHooks, CompactionSource, ProvidedCompaction,
};
use pi_durable::harness::generation::read_live;
use pi_durable::harness::harness::Conversation;
use pi_durable::harness::types::{CompactionReason, HookApi, HookRegistration};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Live, lock};
use crate::print::last_entry;
use crate::sessions::export::{entries_after, short_id};

#[derive(Deserialize)]
struct Before {
    #[serde(default)]
    cancel: bool,
    compaction: Option<Provided>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Provided {
    summary: String,
    first_kept_entry_id: Value,
    tokens_before: u64,
    details: Option<Value>,
    usage: Option<Usage>,
}

fn running(root: &Conversation, cancel: &CancelToken) -> bool {
    let id = root.id();
    root.commit(|tx| read_live(tx, id), cancel)
        .is_ok_and(|live| live.run.is_some())
}

impl Live {
    pub(super) fn compaction_hooks(&self) -> HookRegistration {
        let live = self.clone();
        HookRegistration::new(
            &COMPACTION_HOOK,
            CompactionHooks {
                before_compact: Some(Box::new(move |api: &HookApi, source: &CompactionSource| {
                    Ok(live.before_compact(api, source))
                })),
            },
        )
    }

    fn before_compact(
        &self,
        api: &HookApi,
        source: &CompactionSource,
    ) -> Option<CompactionDecision> {
        let root = self.root_of(api)?;
        let cancel = CancelToken::new();
        if source.reason == CompactionReason::Threshold && !running(&root, &cancel) {
            self.end_run();
        }
        if !self.handles("session_before_compact") {
            return None;
        }
        self.refresh_session(&cancel);
        let policy = &source.policy;
        let answer = self
            .ask(
                "session_before_compact",
                json!({
                    "reason": source.reason,
                    "willRetry": source.reason == CompactionReason::Overflow,
                    "customInstructions": source.instructions,
                    "settings": {
                        "enabled": policy.enabled,
                        "reserveTokens": policy.reserve_tokens,
                        "keepRecentTokens": policy.keep_recent_tokens,
                    },
                }),
            )
            .ok()?;
        let before = match serde_json::from_value::<Option<Before>>(answer) {
            Ok(before) => before?,
            Err(e) => {
                self.0.sink.warn(format!(
                    "the extension host answered session_before_compact with {e}"
                ));
                return None;
            }
        };
        if before.cancel {
            self.compaction_failed(source.reason, None);
            return Some(CompactionDecision::Cancel);
        }
        let provided = before.compaction?;
        let first_kept =
            match crate::sessions::drafts::entry_id(Some(&provided.first_kept_entry_id)) {
                Ok(id) => id,
                Err(e) => {
                    self.0.sink.warn(format!(
                    "a session_before_compact handler kept from an entry pi-rs cannot place: {e}"
                ));
                    return None;
                }
            };
        Some(CompactionDecision::Provided(Box::new(ProvidedCompaction {
            summary: provided.summary,
            first_kept,
            tokens_before: provided.tokens_before,
            details: provided.details,
            usage: provided.usage,
        })))
    }

    pub(super) fn skip_compactions(&self, root: &Conversation, cancel: &CancelToken) {
        if let Ok(last) = last_entry(root, cancel) {
            *lock(&self.0.compacted) = last;
        }
    }

    pub(super) fn report_compactions(&self, root: &Conversation, cancel: &CancelToken) {
        let records = {
            let mut seen = lock(&self.0.compacted);
            let Ok(records) = entries_after(root, *seen, cancel) else {
                return;
            };
            if let Some(last) = records.last() {
                *seen = Some(last.id);
            }
            records
        };
        let drafted = lock(&self.0.run).drafted.clone();
        let kind = compaction_entry();
        for record in records
            .iter()
            .filter(|record| record.kind == kind.kind() && !drafted.contains(&record.id))
        {
            if !self.handles("session_compact") {
                continue;
            }
            let data = record.data.as_ref();
            let reason = data
                .and_then(|data| data.get("reason"))
                .cloned()
                .unwrap_or_else(|| json!(CompactionReason::Manual));
            let from_extension = data
                .and_then(|data| data.get("fromHook"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            self.refresh_session(cancel);
            self.event(
                "session_compact",
                json!({
                    "entryId": short_id(record.id.0),
                    "fromExtension": from_extension,
                    "willRetry": reason == json!(CompactionReason::Overflow),
                    "reason": reason,
                }),
            );
        }
    }

    pub fn wait_compactions(&self) {
        self.0.host.wait_answered("compact");
    }

    pub(super) fn compaction_ended(&self) {
        let _sequence = lock(&self.0.sequence);
        if let Some(root) = self.root() {
            self.report_compactions(&root, &CancelToken::new());
        }
    }

    pub fn compaction_failed(&self, reason: CompactionReason, error: Option<String>) {
        if !self.handles("session_compact_failed") {
            return;
        }
        self.event(
            "session_compact_failed",
            json!({
                "reason": reason,
                "aborted": error.is_none(),
                "errorMessage": error,
                "willRetry": false,
                "fromExtension": false,
            }),
        );
    }
}
