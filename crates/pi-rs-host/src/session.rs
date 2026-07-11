//! `pi.session` — session persistence exposed to Lua.
//!
//! Mechanism only (DESIGN divergence 2): the [`pi_rs_session`] port of
//! `core/session-manager.ts` bound as per-session userdata handles.
//! Policy — which agent events persist, the sdk.ts startup appends,
//! when `/model` or `/name` write entries — lives in the Lua packs
//! (`utils/agent-session.lua` and its consumers).
//!
//! Surface grows with its consumers; today that is the persistence
//! and restore slice (PLAN 6.1/6.2): constructors
//! (`create`/`open`/`in_memory`), the append methods
//! `agent-session.ts` and `sdk.ts` call, and the read-side getters
//! (`build_session_context` feeds the sdk.ts restore in
//! `utils/agent-session.lua`). Listing (`list`/`list_all`, PLAN 6.3)
//! feeds the `/resume` selector; the spec's async progress callback is
//! not bridged — the Rust port lists synchronously, so the transient
//! "Loading …" header state resolves before the next frame. Tree
//! navigation (PLAN 6.4) binds `get_tree`, the branching mutators
//! (`branch`/`reset_leaf`/`branch_with_summary`/`create_branched_session`/
//! `new_session`), and `append_label_change`; compaction (PLAN 6.5)
//! binds `append_compaction`.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, UserData, UserDataMethods, Value};

use pi_rs_session::{NewSessionOptions, SessionManager};

use crate::convert::{json_to_lua, lua_to_json};

pub(crate) struct SessionHandle(Rc<RefCell<SessionManager>>);

fn runtime_err<E: std::fmt::Display>(error: E) -> mlua::Error {
    mlua::Error::runtime(error.to_string())
}

fn opt_string(options: &Table, key: &str) -> mlua::Result<Option<String>> {
    options.get::<Option<String>>(key)
}

/// The spec's `SessionTreeNode` list as Lua tables (`getTree`).
fn tree_nodes_to_lua(lua: &Lua, nodes: Vec<pi_rs_session::SessionTreeNode>) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, node) in nodes.into_iter().enumerate() {
        let row = lua.create_table()?;
        row.set("entry", json_to_lua(lua, &node.entry)?)?;
        row.set("children", tree_nodes_to_lua(lua, node.children)?)?;
        row.set("label", node.label)?;
        row.set("labelTimestamp", node.label_timestamp)?;
        list.set(index + 1, row)?;
    }
    Ok(Value::Table(list))
}

/// The spec's `SessionContext` as a Lua table (`buildSessionContext`).
fn context_to_lua(lua: &Lua, context: pi_rs_session::SessionContext) -> mlua::Result<Value> {
    let table = lua.create_table()?;
    table.set(
        "messages",
        json_to_lua(lua, &serde_json::Value::Array(context.messages))?,
    )?;
    table.set("thinkingLevel", context.thinking_level)?;
    if let Some(model) = context.model {
        let entry = lua.create_table()?;
        entry.set("provider", model.provider)?;
        entry.set("modelId", model.model_id)?;
        table.set("model", entry)?;
    }
    Ok(Value::Table(table))
}

fn entries_to_lua(lua: &Lua, entries: Vec<serde_json::Value>) -> mlua::Result<Value> {
    json_to_lua(lua, &serde_json::Value::Array(entries))
}

/// The spec's `SessionInfo` listing rows as Lua tables. `modified` and
/// `created` are epoch milliseconds (the JS `Date` values' `getTime()`).
fn session_infos_to_lua(lua: &Lua, infos: Vec<pi_rs_session::SessionInfo>) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, info) in infos.into_iter().enumerate() {
        let row = lua.create_table()?;
        row.set("path", info.path.to_string_lossy().into_owned())?;
        row.set("id", info.id)?;
        row.set("cwd", info.cwd)?;
        row.set("name", info.name)?;
        row.set("parentSessionPath", info.parent_session_path)?;
        row.set("created", info.created_ms)?;
        row.set("modified", info.modified_ms)?;
        row.set("messageCount", info.message_count)?;
        row.set("firstMessage", info.first_message)?;
        row.set("allMessagesText", info.all_messages_text)?;
        list.set(index + 1, row)?;
    }
    Ok(Value::Table(list))
}

impl UserData for SessionHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // ---- appends (spec: appendX — append as child of leaf, advance leaf) ----
        methods.add_method("append_message", |_, this, message: Value| {
            let message = lua_to_json(message)?;
            this.0
                .borrow_mut()
                .append_message(message)
                .map_err(runtime_err)
        });
        methods.add_method("append_thinking_level_change", |_, this, level: String| {
            this.0
                .borrow_mut()
                .append_thinking_level_change(&level)
                .map_err(runtime_err)
        });
        methods.add_method(
            "append_model_change",
            |_, this, (provider, model_id): (String, String)| {
                this.0
                    .borrow_mut()
                    .append_model_change(&provider, &model_id)
                    .map_err(runtime_err)
            },
        );
        methods.add_method("append_session_info", |_, this, name: String| {
            this.0
                .borrow_mut()
                .append_session_info(&name)
                .map_err(runtime_err)
        });
        methods.add_method(
            "append_custom_entry",
            |_, this, (custom_type, data): (String, Value)| {
                let data = match data {
                    Value::Nil => None,
                    value => Some(lua_to_json(value)?),
                };
                this.0
                    .borrow_mut()
                    .append_custom_entry(&custom_type, data)
                    .map_err(runtime_err)
            },
        );
        methods.add_method(
            "append_custom_message_entry",
            |_,
             this,
             (custom_type, content, display, details): (
                String,
                Value,
                Option<bool>,
                Value,
            )| {
                let content = lua_to_json(content)?;
                let details = match details {
                    Value::Nil => None,
                    value => Some(lua_to_json(value)?),
                };
                this.0
                    .borrow_mut()
                    .append_custom_message_entry(
                        &custom_type,
                        content,
                        display.unwrap_or(true),
                        details,
                    )
                    .map_err(runtime_err)
            },
        );
        // Spec: `appendCompaction(summary, firstKeptEntryId, tokensBefore,
        // details?, fromHook?)` — the compaction entry the context rebuild
        // cuts over on (PLAN 6.5).
        methods.add_method(
            "append_compaction",
            |_,
             this,
             (summary, first_kept_entry_id, tokens_before, details, from_hook): (
                String,
                String,
                i64,
                Value,
                Option<bool>,
            )| {
                let details = match details {
                    Value::Nil => None,
                    value => Some(lua_to_json(value)?),
                };
                this.0
                    .borrow_mut()
                    .append_compaction(
                        &summary,
                        &first_kept_entry_id,
                        tokens_before,
                        details,
                        from_hook,
                    )
                    .map_err(runtime_err)
            },
        );

        // ---- read side ----
        methods.add_method("get_session_file", |_, this, ()| {
            Ok(this.0.borrow().get_session_file().map(str::to_owned))
        });
        methods.add_method("get_session_id", |_, this, ()| {
            Ok(this.0.borrow().get_session_id().to_owned())
        });
        methods.add_method("get_session_name", |_, this, ()| {
            Ok(this.0.borrow().get_session_name())
        });
        methods.add_method("get_cwd", |_, this, ()| {
            Ok(this.0.borrow().get_cwd().to_owned())
        });
        methods.add_method("get_session_dir", |_, this, ()| {
            Ok(this.0.borrow().get_session_dir().to_owned())
        });
        methods.add_method("get_leaf_id", |_, this, ()| {
            Ok(this.0.borrow().get_leaf_id().map(str::to_owned))
        });
        methods.add_method("is_persisted", |_, this, ()| {
            Ok(this.0.borrow().is_persisted())
        });
        methods.add_method("uses_default_session_dir", |_, this, ()| {
            Ok(this.0.borrow().uses_default_session_dir())
        });
        methods.add_method("get_header", |lua, this, ()| {
            match this.0.borrow().get_header() {
                Some(header) => json_to_lua(lua, header),
                None => Ok(Value::Nil),
            }
        });
        methods.add_method("get_entry", |lua, this, id: String| {
            match this.0.borrow().get_entry(&id) {
                Some(entry) => json_to_lua(lua, entry),
                None => Ok(Value::Nil),
            }
        });
        methods.add_method("get_entries", |lua, this, ()| {
            entries_to_lua(lua, this.0.borrow().get_entries())
        });
        methods.add_method("get_branch", |lua, this, from_id: Option<String>| {
            entries_to_lua(lua, this.0.borrow().get_branch(from_id.as_deref()))
        });
        methods.add_method("build_session_context", |lua, this, ()| {
            context_to_lua(lua, this.0.borrow().build_session_context())
        });
        methods.add_method("get_tree", |lua, this, ()| {
            tree_nodes_to_lua(lua, this.0.borrow().get_tree())
        });
        methods.add_method(
            "export_branch_jsonl",
            |_, this, (output_path, timestamp): (String, String)| {
                this.0
                    .borrow()
                    .export_branch_jsonl(&output_path, &timestamp)
                    .map_err(runtime_err)
            },
        );

        // ---- branching (spec: branch / resetLeaf / branchWithSummary /
        // createBranchedSession / newSession / appendLabelChange) ----
        methods.add_method("branch", |_, this, id: String| {
            this.0.borrow_mut().branch(&id).map_err(runtime_err)
        });
        methods.add_method("reset_leaf", |_, this, ()| {
            this.0.borrow_mut().reset_leaf();
            Ok(())
        });
        methods.add_method(
            "branch_with_summary",
            |_,
             this,
             (from_id, summary, details, from_hook): (
                Option<String>,
                String,
                Value,
                Option<bool>,
            )| {
                let details = match details {
                    Value::Nil => None,
                    value => Some(lua_to_json(value)?),
                };
                this.0
                    .borrow_mut()
                    .branch_with_summary(from_id.as_deref(), &summary, details, from_hook)
                    .map_err(runtime_err)
            },
        );
        methods.add_method("create_branched_session", |_, this, leaf_id: String| {
            this.0
                .borrow_mut()
                .create_branched_session(&leaf_id)
                .map_err(runtime_err)
        });
        methods.add_method("new_session", |_, this, options: Option<Table>| {
            let options = match &options {
                Some(options) => Some(NewSessionOptions {
                    id: opt_string(options, "id")?,
                    parent_session: opt_string(options, "parentSession")?,
                }),
                None => None,
            };
            this.0
                .borrow_mut()
                .new_session(options)
                .map_err(runtime_err)
        });
        methods.add_method(
            "append_label_change",
            |_, this, (target_id, label): (String, Option<String>)| {
                this.0
                    .borrow_mut()
                    .append_label_change(&target_id, label.as_deref())
                    .map_err(runtime_err)
            },
        );
    }
}

fn handle(manager: SessionManager) -> SessionHandle {
    SessionHandle(Rc::new(RefCell::new(manager)))
}

pub(crate) fn install(lua: &Lua, pi: &Table, cwd: &str) -> mlua::Result<()> {
    let table = lua.create_table()?;
    let vm_cwd = cwd.to_owned();

    // Spec: `SessionManager.create(cwd, sessionDir?, options?)` — the
    // default session dir derives from cwd + agent dir (config.ts).
    let default_cwd = vm_cwd.clone();
    table.set(
        "create",
        lua.create_function(move |_, options: Option<Table>| {
            let (cwd, session_dir, agent_dir, new_options) = match &options {
                Some(options) => (
                    opt_string(options, "cwd")?,
                    opt_string(options, "sessionDir")?,
                    opt_string(options, "agentDir")?,
                    Some(NewSessionOptions {
                        id: opt_string(options, "id")?,
                        parent_session: opt_string(options, "parentSession")?,
                    }),
                ),
                None => (None, None, None, None),
            };
            let cwd = cwd.unwrap_or_else(|| default_cwd.clone());
            let agent_dir = agent_dir.unwrap_or_else(crate::discover::agent_dir);
            let manager =
                SessionManager::create(&cwd, session_dir.as_deref(), &agent_dir, new_options)
                    .map_err(runtime_err)?;
            Ok(handle(manager))
        })?,
    )?;

    // Spec: `SessionManager.open(sessionPath, sessionDir?, cwdOverride?)`
    // — without an override the cwd comes from the session header.
    table.set(
        "open",
        lua.create_function(move |_, options: Table| {
            let path: String = options.get("path")?;
            let session_dir = opt_string(&options, "sessionDir")?;
            let cwd = opt_string(&options, "cwd")?;
            let agent_dir =
                opt_string(&options, "agentDir")?.unwrap_or_else(crate::discover::agent_dir);
            let manager =
                SessionManager::open(&path, session_dir.as_deref(), cwd.as_deref(), &agent_dir)
                    .map_err(runtime_err)?;
            Ok(handle(manager))
        })?,
    )?;

    // Spec: `SessionManager.list(cwd, sessionDir?)` — sessions for a
    // project directory, most recently modified first.
    let list_cwd = vm_cwd.clone();
    table.set(
        "list",
        lua.create_function(move |lua, options: Option<Table>| {
            let (cwd, session_dir, agent_dir) = match &options {
                Some(options) => (
                    opt_string(options, "cwd")?,
                    opt_string(options, "sessionDir")?,
                    opt_string(options, "agentDir")?,
                ),
                None => (None, None, None),
            };
            let cwd = cwd.unwrap_or_else(|| list_cwd.clone());
            let agent_dir = agent_dir.unwrap_or_else(crate::discover::agent_dir);
            let sessions = SessionManager::list(&cwd, session_dir.as_deref(), &agent_dir, None)
                .map_err(runtime_err)?;
            session_infos_to_lua(lua, sessions)
        })?,
    )?;

    // Spec: `SessionManager.listAll(sessionDir?)` — every session across
    // all project directories (`{agentDir}/sessions`), or one custom
    // flat directory.
    table.set(
        "list_all",
        lua.create_function(move |lua, options: Option<Table>| {
            let (session_dir, agent_dir) = match &options {
                Some(options) => (
                    opt_string(options, "sessionDir")?,
                    opt_string(options, "agentDir")?,
                ),
                None => (None, None),
            };
            let agent_dir = agent_dir.unwrap_or_else(crate::discover::agent_dir);
            let default_dir = std::path::Path::new(&pi_rs_session::paths::resolve_path(&agent_dir))
                .join("sessions");
            let sessions = SessionManager::list_all(session_dir.as_deref(), &default_dir, None);
            session_infos_to_lua(lua, sessions)
        })?,
    )?;

    // Spec: `SessionManager.inMemory(cwd?)` — never persists.
    table.set(
        "in_memory",
        lua.create_function(move |_, options: Option<Table>| {
            let cwd = match &options {
                Some(options) => opt_string(options, "cwd")?,
                None => None,
            };
            let cwd = cwd.unwrap_or_else(|| vm_cwd.clone());
            Ok(handle(SessionManager::in_memory_at(&cwd)))
        })?,
    )?;

    // Spec: the standalone `buildSessionContext(entries)` — compaction
    // (PLAN 6.5) computes `tokensBefore` over the current branch entries
    // without re-reading the session file.
    table.set(
        "build_context",
        lua.create_function(|lua, entries: Table| {
            let entries: Vec<serde_json::Value> = entries
                .sequence_values::<Value>()
                .map(|value| value.and_then(lua_to_json))
                .collect::<mlua::Result<_>>()?;
            context_to_lua(
                lua,
                pi_rs_session::build_session_context(&entries, pi_rs_session::Leaf::Latest),
            )
        })?,
    )?;

    // messages.ts timestamp semantics: JS `Date.parse` of the entry ISO
    // string (`NaN` → nil), for the Lua-side message constructors
    // (`getMessageFromEntry` in the branch-summarization port).
    table.set(
        "parse_iso_ms",
        lua.create_function(|_, timestamp: String| {
            Ok(pi_rs_session::time::parse_iso_ms(&timestamp))
        })?,
    )?;

    pi.set("session", table)?;
    Ok(())
}
