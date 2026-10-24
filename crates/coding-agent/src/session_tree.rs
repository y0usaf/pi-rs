//! `core/session-manager.ts`'s session tree on pi-durable: a conversation is one branch of the
//! session, so every conversation in the file contributes its entries to one tree.

use std::collections::{BTreeMap, BTreeSet};

use pi_durable::cancel::CancelToken;
use pi_durable::errors::Error;
use pi_durable::harness::harness::{Conversation, Harness};
use pi_durable::harness::types::{ConversationCreateOptions, RootOptions};
use pi_durable::ids::{ConversationId, EntryId};
use pi_durable::types::{
    ConversationOwnership, ConversationQuery, Cursor, EntryQuery, EntryRecord, ScanOrder,
};

const PAGE: usize = 256;

pub struct TreeNode {
    pub entry: EntryRecord,
    pub conversation: ConversationId,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

pub struct Tree {
    pub nodes: Vec<TreeNode>,
    pub roots: Vec<usize>,
    pub leaf: Option<EntryId>,
    pub active: BTreeSet<EntryId>,
    index: BTreeMap<EntryId, usize>,
}

impl Tree {
    pub fn build(conversation: &Conversation, cancel: &CancelToken) -> Result<Self, Error> {
        let mut nodes: Vec<TreeNode> = Vec::new();
        let mut index: BTreeMap<EntryId, usize> = BTreeMap::new();
        let mut leaf: Option<EntryId> = None;
        let active_id = conversation.id();
        for record in conversations(conversation, cancel)? {
            let own = own_entries(conversation, record.id, cancel)?;
            let mut previous: Option<EntryId> = None;
            let mut last: Option<EntryId> = None;
            for entry in own {
                let attach = previous.or_else(|| record.parent.as_ref().map(|parent| parent.at));
                let parent = attach.and_then(|id| index.get(&id).copied());
                let position = nodes.len();
                if let Some(parent) = parent {
                    nodes[parent].children.push(position);
                }
                index.insert(entry.id, position);
                last = Some(entry.id);
                nodes.push(TreeNode {
                    entry,
                    conversation: record.id,
                    parent,
                    children: Vec::new(),
                });
                previous = last;
            }
            if record.id == active_id {
                leaf = last.or_else(|| record.parent.as_ref().map(|parent| parent.at));
            }
        }
        if leaf.is_some_and(|id| !index.contains_key(&id)) {
            leaf = None;
        }
        let roots: Vec<usize> = (0..nodes.len())
            .filter(|i| nodes[*i].parent.is_none())
            .collect();
        let mut active = BTreeSet::new();
        let mut cursor = leaf.and_then(|id| index.get(&id).copied());
        while let Some(i) = cursor {
            active.insert(nodes[i].entry.id);
            cursor = nodes[i].parent;
        }
        Ok(Self {
            nodes,
            roots,
            leaf,
            active,
            index,
        })
    }

    pub fn node(&self, id: EntryId) -> Option<&TreeNode> {
        self.index.get(&id).map(|i| &self.nodes[*i])
    }

    pub fn entry(&self, id: EntryId) -> Option<&EntryRecord> {
        self.node(id).map(|node| &node.entry)
    }

    pub fn has(&self, id: EntryId) -> bool {
        self.index.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn path_to(&self, id: EntryId) -> Vec<EntryId> {
        let mut path = Vec::new();
        let mut cursor = self.index.get(&id).copied();
        while let Some(i) = cursor {
            path.push(self.nodes[i].entry.id);
            cursor = self.nodes[i].parent;
        }
        path.reverse();
        path
    }
}

/// Move the session's active entry to `target`: pi-durable branches by forking the conversation
/// that holds the target, so the abandoned entries stay in the file while the context ends at
/// the target. `None` when the target is already the leaf.
pub fn navigate(
    harness: &Harness,
    current: &Conversation,
    target: Option<EntryId>,
    cancel: &CancelToken,
) -> Result<Option<Conversation>, Error> {
    let tree = Tree::build(current, cancel)?;
    let Some(target) = target else {
        if tree.leaf.is_none() {
            return Ok(None);
        }
        let empty = harness.create_conversation(
            ConversationCreateOptions {
                ownership: ConversationOwnership::Ownerless,
                agent: None,
                init: None,
            },
            cancel,
        )?;
        return Ok(Some(empty));
    };
    if tree.leaf == Some(target) {
        return Ok(None);
    }
    let Some(node) = tree.node(target) else {
        return Err(Error::Invalid(format!(
            "Entry {target} is not in this session"
        )));
    };
    let owner = harness
        .conversation(node.conversation, cancel)?
        .ok_or_else(|| Error::Invalid(format!("Conversation {} is gone", node.conversation)))?;
    let branch = owner.fork(
        target,
        ConversationCreateOptions {
            ownership: ConversationOwnership::Ownerless,
            agent: None,
            init: None,
        },
        cancel,
    )?;
    Ok(Some(branch))
}

/// The text of an entry that is a user message: navigating to one forks before it and puts this
/// in the editor, as upstream's navigateTree does.
pub fn user_message_text(entry: &EntryRecord) -> Option<String> {
    if entry.kind != pi_durable::entries::user_entry().kind() {
        return None;
    }
    let Some(pi_durable::ai::types::Message::User(user)) = entry.model.as_ref()?.first() else {
        return None;
    };
    Some(match &user.content {
        pi_durable::ai::types::UserContent::Text(text) => text.clone(),
        pi_durable::ai::types::UserContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                pi_durable::ai::types::UserBlock::Text(content) => Some(content.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
    })
}

/// The entry a navigation lands before: a user message is re-opened in the editor, so the leaf
/// moves to its parent (nothing, at the first entry).
pub fn fork_point(tree: &Tree, target: EntryId) -> Option<EntryId> {
    let node = tree.node(target)?;
    if user_message_text(&node.entry).is_none() {
        return Some(target);
    }
    node.parent.map(|parent| tree.nodes[parent].entry.id)
}

/// The conversation holding the newest entry, which is where a session continues.
pub fn active_conversation(harness: &Harness, cancel: &CancelToken) -> Result<Conversation, Error> {
    let root = harness.root(RootOptions::default(), cancel)?;
    let mut newest: Option<(EntryId, ConversationId)> = None;
    for record in conversations(&root, cancel)? {
        if let Some(last) = own_entries(&root, record.id, cancel)?.last()
            && newest.is_none_or(|(id, _)| last.id > id)
        {
            newest = Some((last.id, record.id));
        }
    }
    match newest {
        Some((_, id)) if id != root.id() => match harness.conversation(id, cancel)? {
            Some(conversation) => Ok(conversation),
            None => Ok(root),
        },
        _ => Ok(root),
    }
}

fn conversations(
    conversation: &Conversation,
    cancel: &CancelToken,
) -> Result<Vec<pi_durable::types::ConversationRecord>, Error> {
    let query = ConversationQuery {
        owner_conversation_id: None,
        owner_task_id: None,
        order: Some(ScanOrder::Ascending),
    };
    conversation.commit(
        |tx| {
            let mut items = Vec::new();
            let mut cursor: Option<Cursor> = None;
            loop {
                let page = tx.scan_conversations(&query, PAGE, cursor.as_ref())?;
                items.extend(page.items);
                match page.next {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            Ok(items)
        },
        cancel,
    )
}

/// A conversation's own entries: a scan stitches its ancestors in, and those are another
/// branch's.
fn own_entries(
    conversation: &Conversation,
    id: ConversationId,
    cancel: &CancelToken,
) -> Result<Vec<EntryRecord>, Error> {
    let query = EntryQuery {
        conversation_id: id,
        min_entry_id: None,
        max_entry_id: None,
        order: Some(ScanOrder::Ascending),
    };
    conversation.commit(
        |tx| {
            let mut items = Vec::new();
            let mut cursor: Option<Cursor> = None;
            loop {
                let page = tx.scan_entries(&query, PAGE, cursor.as_ref())?;
                items.extend(
                    page.items
                        .into_iter()
                        .filter(|entry| entry.conversation_id == id),
                );
                match page.next {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            Ok(items)
        },
        cancel,
    )
}
