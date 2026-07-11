-- Exerciser for the session bindings: pi.session.create/open/in_memory
-- handles over pi-rs-session's port of core/session-manager.ts.
--
-- In pi this is the SessionManager the coding agent persists through —
-- sessions as append-only JSONL trees, file creation deferred until the
-- first assistant message lands (`_persist`). The product policy that
-- decides *what* to append lives in the builtin packs
-- (utils/agent-session.lua); this demo drives the mechanism directly.
local pi = ...

pi.register_command("session-demo", {
  description = "Walk the session persistence bindings",
  handler = function(args)
    local request = pi.json.decode(args)
    local session = pi.session.create({
      cwd = request.cwd, sessionDir = request.sessionDir, agentDir = request.agentDir,
    })

    -- sdk.ts startup appends for a new session.
    session:append_model_change("demo-provider", "demo-model")
    session:append_thinking_level_change("off")

    -- No assistant message yet: the file stays unwritten (spec _persist).
    local before_assistant = session:get_session_file()
    local deferred = not pi.fs.exists(before_assistant)

    session:append_message({
      role = "user",
      content = { { type = "text", text = "hello" } },
      timestamp = 0,
    })
    session:append_message({
      role = "assistant",
      content = { { type = "text", text = "hi there" } },
      api = "demo-api", provider = "demo-provider", model = "demo-model",
      usage = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0, totalTokens = 0,
        cost = { input = 0, output = 0, cacheRead = 0, cacheWrite = 0, total = 0 } },
      stopReason = "stop", timestamp = 0,
    })
    session:append_session_info("demo session")

    -- AgentSession.exportToJsonl's persistence mechanism: current branch
    -- only, with parent IDs re-chained into one linear sequence.
    local exported = session:export_branch_jsonl(
      request.exportPath, "2026-07-11T12:34:56.789Z")

    local context = session:build_session_context()
    local reopened = pi.session.open({
      path = session:get_session_file(),
      cwd = request.cwd, sessionDir = request.sessionDir, agentDir = request.agentDir,
    })

    local in_memory = pi.session.in_memory({ cwd = request.cwd })
    in_memory:append_message({
      role = "assistant", content = { { type = "text", text = "never on disk" } },
      timestamp = 0,
    })

    -- Listing (SessionManager.list/listAll — the /resume selector's data):
    -- rows carry the spec's SessionInfo fields, most recently modified
    -- first; list_all walks every project dir (or one custom dir).
    local listed = pi.session.list({
      cwd = request.cwd, sessionDir = request.sessionDir, agentDir = request.agentDir,
    })
    local listed_all = pi.session.list_all({
      sessionDir = request.sessionDir, agentDir = request.agentDir,
    })

    -- Branching (the /tree, /fork, and /clone mechanism — PLAN 6.4), on
    -- an in-memory session so the persisted walkthrough above stays
    -- untouched: labels resolve onto tree nodes, branch() moves the leaf
    -- so the next append forks, branch_with_summary records the abandoned
    -- path, and create_branched_session copies root→leaf (no file in
    -- memory mode).
    local branching = pi.session.in_memory({ cwd = request.cwd })
    local user_id = branching:append_message({
      role = "user", content = { { type = "text", text = "root prompt" } }, timestamp = 0,
    })
    branching:append_message({
      role = "assistant", content = { { type = "text", text = "take one" } }, timestamp = 0,
    })
    branching:append_label_change(user_id, "important")
    branching:branch(user_id)
    branching:append_message({
      role = "assistant", content = { { type = "text", text = "take two" } }, timestamp = 0,
    })
    local tree = branching:get_tree()
    local labeled_node
    local function find_labeled(nodes)
      for _, node in ipairs(nodes) do
        if node.label then labeled_node = node end
        find_labeled(node.children)
      end
    end
    find_labeled(tree)
    local branch_children = #tree[1].children
    local summary_id = branching:branch_with_summary(user_id,
      "Explored a second take.", { readFiles = {}, modifiedFiles = {} }, false)
    local summary_is_leaf = branching:get_leaf_id() == summary_id
    local summary_entry = branching:get_entry(summary_id)
    -- In-memory mode rebuilds the session in place and returns no path.
    local branched_file = branching:create_branched_session(user_id)

    -- Compaction (the /compact mechanism — PLAN 6.5): append_compaction
    -- records the summary + cut point, and build_session_context cuts
    -- over — the summary message replaces everything before the kept
    -- entry. pi.session.build_context is the standalone
    -- buildSessionContext over raw entries (the compaction policy
    -- computes tokensBefore with it).
    local compacting = pi.session.in_memory({ cwd = request.cwd })
    compacting:append_message({
      role = "user", content = { { type = "text", text = "old prompt" } }, timestamp = 0,
    })
    compacting:append_message({
      role = "assistant", content = { { type = "text", text = "old answer" } }, timestamp = 0,
    })
    local kept_id = compacting:append_message({
      role = "user", content = { { type = "text", text = "recent prompt" } }, timestamp = 0,
    })
    compacting:append_compaction("What came before.", kept_id, 1234,
      { readFiles = {}, modifiedFiles = {} }, false)
    local compacted_context = compacting:build_session_context()
    local standalone_context = pi.session.build_context(compacting:get_entries())

    return {
      treeRoots = #tree,
      branchChildren = branch_children,
      labeledEntry = labeled_node and labeled_node.entry.id,
      labeledEntryIsUser = labeled_node and labeled_node.entry.id == user_id,
      labeledLabel = labeled_node and labeled_node.label,
      summaryLeaf = summary_is_leaf,
      summarySummary = summary_entry and summary_entry.summary,
      summaryFromId = summary_entry and summary_entry.fromId,
      branchedFile = branched_file,
      isoMs = pi.session.parse_iso_ms("2026-07-01T10:00:00.000Z"),
      listedCount = #listed,
      listedAllCount = #listed_all,
      listedName = listed[1] and listed[1].name,
      listedFirstMessage = listed[1] and listed[1].firstMessage,
      listedMessageCount = listed[1] and listed[1].messageCount,
      usesDefaultSessionDir = session:uses_default_session_dir(),
      sessionFile = session:get_session_file(),
      exportedFile = exported,
      deferredUntilAssistant = deferred,
      sessionId = session:get_session_id(),
      leafId = session:get_leaf_id(),
      name = session:get_session_name(),
      entryCount = #session:get_entries(),
      branchCount = #session:get_branch(),
      contextMessages = #context.messages,
      contextModel = context.model,
      reopenedId = reopened:get_session_id(),
      reopenedName = reopened:get_session_name(),
      inMemoryPersisted = in_memory:is_persisted(),
      inMemoryFile = in_memory:get_session_file(),
      compactedMessages = #compacted_context.messages,
      compactedFirstRole = compacted_context.messages[1]
        and compacted_context.messages[1].role,
      compactedSummary = compacted_context.messages[1]
        and compacted_context.messages[1].summary,
      standaloneMessages = #standalone_context.messages,
    }
  end,
})
