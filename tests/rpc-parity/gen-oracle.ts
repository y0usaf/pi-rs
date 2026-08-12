// Generates tests/rpc-parity/oracle.json by driving Pi's real `runRpcMode`
// (modes/rpc/rpc-mode.ts) with a scripted session/runtime stub, the same
// technique used by modes/print-mode.ts. impoorting the full coding-agent
// runtime (which pulls jiti) fails under the tsx register, so the stub
// reproduces every surface runRpcMode touches: session state getters,
// session.prompt/steer/followUp/abort, setModel/cycleModel, thinking/queue
// mode setters, compaction/retry, bash, session actions, modelRegistry,
// extensionRunner/promptTemplates/resourceLoader, session.subscribe, and
// runtime.setRebindSession/dispose.
//
// Each case writes a fixed sequence of RPC commands to a fake process.stdin
// (LF-only JSONL, exactly as attachJsonlLineReader frames), then returns the
// process's raw stdout bytes as a CLI caller / the RpcClient would observe
// them. The scripted session answers each command deterministically.
//
// Run via scripts/rpc-oracle. Offline normal checks consume the checked
// oracle; opt-in regeneration drives the pinned Pi source.
import { runRpcMode } from "../../ref/pi/packages/coding-agent/src/modes/rpc/rpc-mode.ts";
import { serializeJsonLine } from "../../ref/pi/packages/coding-agent/src/modes/rpc/jsonl.ts";
import { writeFileSync } from "node:fs";

type Json = any;

/** A minimal scripted session exposing every getter/method `runRpcMode` reads. */
function makeSession(seed: {
  model?: Json;
  thinkingLevel?: string;
  steeringMode?: "all" | "one-at-a-time";
  followUpMode?: "all" | "one-at-a-time";
  sessionFile?: string;
  sessionName?: string;
  autoCompaction?: boolean;
  messages?: Json[];
  pendingCount?: number;
  availableModels?: Json[];
  registeredCommands?: Json[];
  promptTemplates?: Json[];
  skills?: Json[];
  bashResult?: Json;
  sessionStats?: Json;
  forkingMessages?: Json[];
  lastAssistantText?: string | null;
  promptDelayMs?: number;
  promptError?: string;
  compactResult?: Json;
  forkMessages?: Json[];
}): { session: Json; subs: Array<(e: any) => void> } {
  const subs: Array<(e: any) => void> = [];
  let currentModel = seed.model;
  let currentThinking = seed.thinkingLevel ?? "medium";
  let cycleCount = 0;
  let promptCalls = 0;

  const session: any = {
    agent: { subscribe: () => () => {} },
    model: currentModel,
    subscribe: (fn: any) => {
      subs.push(fn);
      return () => {
        const i = subs.indexOf(fn);
        if (i !== -1) subs.splice(i, 1);
      };
    },
    get thinkingLevel() { return currentThinking; },
    get isStreaming() { return false; },
    get isCompacting() { return false; },
    get steeringMode() { return seed.steeringMode ?? "one-at-a-time"; },
    get followUpMode() { return seed.followUpMode ?? "one-at-a-time"; },
    get sessionFile() { return seed.sessionFile; },
    get sessionId() { return "sid-123"; },
    get sessionName() { return seed.sessionName; },
    get autoCompactionEnabled() { return seed.autoCompaction ?? false; },
    get messages() { return seed.messages ?? []; },
    get pendingMessageCount() { return seed.pendingCount ?? 0; },
    async bindExtensions(opts: any) {
      // capture handlers for extension-UI / command-context tests
      (session as any)._bound = opts;
    },
    async prompt(msg: string, opts?: any) {
      promptCalls++;
      const delay = seed.promptDelayMs ?? 0;
      if (delay > 0) await new Promise((r) => setTimeout(r, delay));
      const cb = opts?.preflightResult;
      if (seed.promptError && promptCalls >= (seed.promptErrorOnCall ?? 1)) {
        if (cb) cb(false);
        throw new Error(seed.promptError);
      }
      if (cb) cb(true);
    },
    async steer() {},
    async followUp() {},
    async abort() {},
    async compact() { return seed.compactResult ?? {}; },
    setThinkingLevel(l: string) { currentThinking = l; },
    cycleThinkingLevel() {
      // Pi's AgentSession.cycleThinkingLevel() returns the selected ThinkingLevel
      // *string*; rpc-mode wraps it as `data: { level }`.
      const levels = ["off", "low", "medium", "high"];
      const i = levels.indexOf(currentThinking);
      const next = levels[(i + 1) % levels.length];
      currentThinking = next;
      return currentThinking;
    },
    setSteeringMode() {},
    setFollowUpMode() {},
    setAutoCompactionEnabled() {},
    setAutoRetryEnabled() {},
    abortRetry() {},
    async executeBash() { return seed.bashResult ?? { exitCode: 0, stdout: "", stderr: "" }; },
    abortBash() {},
    getSessionStats() { return seed.sessionStats ?? {}; },
    async exportToHtml() { return "/tmp/export.html"; },
    async navigateTree() { return { cancelled: false }; },
    async reload() {},
    sessionManager: {
      getLeafId: () => "leaf-1",
      getHeader: () => null,
    },
    getUserMessagesForForking() { return seed.forkingMessages ?? []; },
    getLastAssistantText() { return seed.lastAssistantText ?? null; },
    setSessionName() {},
    extensionRunner: { getRegisteredCommands: () => seed.registeredCommands ?? [] },
    promptTemplates: seed.promptTemplates ?? [],
    resourceLoader: { getSkills: () => ({ skills: seed.skills ?? [] }) },
    modelRegistry: {
      async getAvailable() { return seed.availableModels ?? []; },
    },
    async setModel(m: Json) { currentModel = m; },
    cycleModel() {
      const models = seed.availableModels ?? [];
      if (models.length === 0) return null;
      const m = models[cycleCount % models.length];
      cycleCount++;
      return { model: m, thinkingLevel: currentThinking, isScoped: false };
    },
  };
  return { session, subs };
}

/**
 * Drive runRpcMode against a corpus of stdin JSONL lines and return the raw
 * stdout bytes (exactly what the RpcClient / a CLI caller reads). We install a
 * fake stdin + stdout capture before calling runRpcMode, then trigger an
 * orderly shutdown on stdin end (mirroring a real EOF).
 */
async function drive(
  lines: Json[],
  seed: Parameters<typeof makeSession>[0],
  rawInput?: string,
): Promise<{ stdout: string; timedOut: boolean }> {
  const input = rawInput ?? lines.map(serializeJsonLine).join("");
  (process as any)._writeLog = [];

  // stdout takeover capture.
  const origStdout = process.stdout.write.bind(process.stdout);
  (process.stdout as any).write = (chunk: any, enc?: any, cb?: any) => {
    (process as any)._writeLog.push(String(chunk));
    if (typeof enc === "function") enc(); else if (typeof cb === "function") cb();
    return true;
  };
  const origStderr = process.stderr.write.bind(process.stderr);
  (process.stderr as any).write = (chunk: any, enc?: any, cb?: any) => {
    if (typeof enc === "function") enc(); else if (typeof cb === "function") cb();
    return true;
  };

  const { session, subs } = makeSession(seed);
  const runtime: any = {
    session,
    setRebindSession: () => {},
    async dispose() {},
    async newSession(opts?: any) { session.sessionManager.getLeafId(); return { cancelled: false }; },
    async fork() { return { cancelled: false, selectedText: "" }; },
    async switchSession() { return { cancelled: false }; },
  };

  // Fake stdin emitting all lines then EOF. Support multiple listeners per
  // event (rpc-mode registers both a reader "end" and its own "end"→shutdown).
  const listeners: any = {};
  const add = (ev: string, fn: any) => {
    (listeners[ev] ??= []).push(fn);
  };
  let dataEnabled = false;
  (process as any)._debug = [];
  const fakeStdin: any = {
    on(ev: string, fn: any) {
      (process as any)._debug.push("on:" + ev);
      add(ev, fn);
      if (ev === "data" && !dataEnabled) {
        dataEnabled = true;
        setTimeout(() => {
          (process as any)._debug.push("emitting:" + (listeners.data ? "yes" : "no"));
          for (const l of listeners.data ?? []) l(input);
          (process as any)._debug.push("schedule-end");
          setTimeout(() => {
            for (const l of listeners.end ?? []) l();
          }, 5);
        }, 10);
      }
    },
    once(ev: string, fn: any) {
      add(ev, fn);
    },
    off() { return this; },
    pause() {},
    isTTY: false,
  };
  Object.defineProperty(process, "stdin", {
    value: fakeStdin,
    configurable: true,
    writable: true,
  });

  let exitCode = 0;
  // Prevent the process from actually exiting out from under the capture; we
  // model Pi's process.exit on shutdown by ending the run.
  let runExited = false;
  const origExit = process.exit.bind(process) as any;
  process.exit = ((code?: any) => {
    exitCode = code ?? 0;
    runExited = true;
    // Don't hard-exit: simulate after capture.
  }) as any;

  let timedOut = false;
  // runRpcMode never resolves (keeps process alive); observe via timeout.
  const p: Promise<any> = runRpcMode(runtime as any);
  // Wait for command processing + raw-stdout tail to flush: poll until writes
  // stop growing for a quiet period, bounded.
  const deadline = Date.now() + 2000;
  let lastLen = 0;
  while (Date.now() < deadline) {
    await new Promise((res) => setTimeout(res, 20));
    const len = (process as any)._writeLog.length;
    if (len !== 0 && len === lastLen) break;
    if (len !== lastLen) lastLen = len;
  }
  if (!runExited) {
    timedOut = true;
  }

  (process.stdout as any).write = origStdout;
  (process.stderr as any).write = origStderr;
  process.exit = origExit;
  const content = (process as any)._writeLog.join("");
  return { stdout: content, timedOut, debug: (process as any)._debug, writes: (process as any)._writeLog };
}

export async function main(): Promise<void> {
  const model = {
    provider: "anthropic", id: "claude-3-5-sonnet", api: "anthropic-messages",
    baseUrl: "", reasoning: true,
  };
  const out: Json = { oracle: "Pi v0.79.0 c5582102", rpc: true, cases: [] as Json[] };

  const cases: Array<{ name: string; lines: Json[]; seed: Parameters<typeof makeSession>[0] }> = [
    {
      name: "state-and-simple",
      lines: [
        { type: "get_state", id: "r1" },
        { type: "get_available_models", id: "r2" },
        { type: "set_steering_mode", mode: "all", id: "r3" },
        { type: "set_follow_up_mode", mode: "one-at-a-time", id: "r4" },
        { type: "set_auto_compaction", enabled: true, id: "r5" },
        { type: "set_auto_retry", enabled: true, id: "r6" },
        { type: "abort_retry", id: "r7" },
        { type: "get_messages", id: "r8" },
        { type: "get_last_assistant_text", id: "r9" },
      ],
      seed: {
        model, availableModels: [model], messages: [
          { role: "user", content: [{ type: "text", text: "hi" }], timestamp: 0 },
        ],
        lastAssistantText: "hello",
      },
    },
    {
      name: "unknown-command-no-id",
      lines: [
        { type: "definitely_not_a_command", id: "r1" },
      ],
      seed: {},
    },
    {
      name: "prompt-async-success",
      lines: [
        { type: "prompt", message: "build it", id: "r1" },
        { type: "get_state", id: "r2" },
      ],
      seed: { model, promptDelayMs: 5 },
    },
    {
      name: "prompt-preflight-failure",
      lines: [
        { type: "prompt", message: "nope", id: "r1" },
      ],
      seed: { promptError: "preflight rejected", promptErrorOnCall: 1 },
    },
    {
      name: "thinking-model-commands",
      lines: [
        { type: "set_thinking_level", level: "high", id: "r1" },
        { type: "cycle_thinking_level", id: "r2" },
        { type: "set_model", provider: "anthropic", modelId: "claude-3-5-sonnet", id: "r3" },
        { type: "cycle_model", id: "r4" },
        { type: "set_thinking_level", level: "off", id: "r5" },
      ],
      seed: { model, availableModels: [model, { ...model, id: "claude-3-haiku" }] },
    },
    {
      name: "set-model-not-found",
      lines: [
        { type: "set_model", provider: "x", modelId: "nope", id: "r1" },
      ],
      seed: { availableModels: [model] },
    },
    {
      name: "compact-bash-session-ops",
      lines: [
        { type: "compact", customInstructions: "summarize", id: "r1" },
        { type: "bash", command: "ls", excludeFromContext: false, id: "r2" },
        { type: "abort_bash", id: "r3" },
        { type: "abort", id: "r4" },
        { type: "steer", message: "go left", id: "r5" },
        { type: "follow_up", message: "and right", id: "r6" },
        { type: "get_session_stats", id: "r7" },
      ],
      seed: {
        compactResult: { sessionId: "sid-123", summary: "sum", kept: 3 },
        bashResult: { exitCode: 0, stdout: "a.txt\nb.txt", stderr: "" },
        sessionStats: { messageCount: 2, tokenCount: 100, sessionId: "sid-123" },
      },
    },
    {
      name: "export-html",
      lines: [
        { type: "export_html", outputPath: "/out.html", id: "r1" },
        { type: "get_session_stats", id: "r2" },
      ],
      seed: {},
    },
    {
      name: "session-fork-clone",
      lines: [
        { type: "new_session", id: "r1" },
        { type: "fork", entryId: "entry-1", id: "r2" },
        { type: "clone", id: "r3" },
        { type: "get_fork_messages", id: "r4" },
        { type: "set_session_name", name: "my session", id: "r5" },
        { type: "set_session_name", name: "   ", id: "r6" },
      ],
      seed: { forkingMessages: [{ entryId: "entry-1", text: "fork me" }], sessionFile: "/s.jsonl" },
    },
    {
      name: "commands-registry",
      lines: [
        { type: "get_commands", id: "r1" },
        { type: "get_last_assistant_text", id: "r2" },
      ],
      seed: {
        registeredCommands: [
          { invocationName: "slash-cmd", description: "a cmd", sourceInfo: { path: "/x.lua" } },
        ],
        promptTemplates: [
          { name: "review", description: "review prompt", sourceInfo: { path: "/review.lua" } },
        ],
        skills: [
          { name: "web", description: "web skill", sourceInfo: { path: "/web.lua" } },
        ],
        lastAssistantText: null,
      },
    },
    {
      name: "event-streaming",
      lines: [
        { type: "prompt", message: "tell me", id: "r1" },
      ],
      seed: { model, promptDelayMs: 2 },
    },
    {
      name: "async-steer-followup-abort",
      lines: [
        { type: "steer", message: "go left", id: "r1" },
        { type: "follow_up", message: "and right", id: "r2" },
        { type: "abort", id: "r3" },
        { type: "abort_bash", id: "r4" },
      ],
      seed: { model },
    },
    {
      name: "fork-messages",
      lines: [
        { type: "get_fork_messages", id: "r1" },
      ],
      seed: { forkingMessages: [{ entryId: "entry-1", text: "fork me" }] },
    },
    {
      name: "empty-fork-messages",
      lines: [
        { type: "get_fork_messages", id: "r1" },
      ],
      // No forkingMessages: Pi's session returns an empty array. Pins the
      // empty-array serialization pi-rs previously got wrong (empty Lua table
      // → object {} instead of []).
      seed: {},
    },
    {
      name: "empty-commands",
      lines: [
        { type: "get_commands", id: "r1" },
      ],
      // No extension commands/prompts/skills: Pi's get_commands returns a real
      // empty array. Pins the empty-array serialization pi-rs got wrong.
      seed: {},
    },
    {
      name: "empty-messages",
      lines: [
        { type: "get_messages", id: "r1" },
      ],
      // No messages: Pi's session.messages is a real empty array, so data is
      // {messages: []}. Pins the empty-array serialization pi-rs got wrong
      // (empty Lua table → {}).
      seed: {},
    },
  ];

  // parse-error case is driven with a raw non-JSON line.
  const parseOut = await drive([], {}, "not json at all\n");
  out.parseErrorProbe = { stdout: parseOut.stdout, timedOut: parseOut.timedOut };

  for (const c of cases) {
    const r = await drive(c.lines, c.seed);
    out.cases.push({
      name: c.name,
      commands: c.lines.map((l) => {
        // canonicalize: drop undefined id so record reflects what's serialized
        const o: any = {};
        for (const [k, v] of Object.entries(l)) if (v !== undefined) o[k] = v;
        return o;
      }),
      seed: c.seed,
      stdout: r.stdout,
      timedOut: r.timedOut,
    });
  }

  writeFileSync(process.argv[2]!, JSON.stringify(out, null, 2) + "\n");
  console.log(`wrote ${process.argv[2]}`);
}
main().catch((e) => { throw e; });
