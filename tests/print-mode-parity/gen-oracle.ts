// Generates tests/print-mode-parity/oracle.json by driving Pi's real
// `runPrintMode` (modes/print-mode.ts) without importing the full coding-agent
// runtime (which would pull jiti and fail under the tsx register). Instead a
// scripted session/stub runtime reproduces the exact surfaces print-mode
// touches: session.prompt, session.state, session.sessionManager.getHeader,
// session.subscribe, session.bindExtensions, and runtime.dispose.
//
// For each case we capture Pi's raw stdout bytes and process exit code the way
// a CLI caller would observe them, so pi-rs's print/text and JSON mode output
// and exit status can be differentially verified byte-for-byte.
//
// Run via scripts/print-mode-oracle. Offline normal checks consume the checked
// oracle; opt-in regeneration drives the pinned Pi source.
import type { AgentMessage, AssistantMessage } from "pi-agent-core";
import { runPrintMode } from "../../ref/pi/packages/coding-agent/src/modes/print-mode.ts";
import {
  flushRawStdout,
  restoreStdout,
  takeOverStdout,
} from "../../ref/pi/packages/coding-agent/src/core/output-guard.ts";
import { writeFileSync } from "node:fs";

type Json = any;

const EMPTY_USAGE = {
  input: 0, output: 0, cacheRead: 0, cacheWrite: 0,
  totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
};

/** A controlled capture of stdout/stderr/exit like `process` would report. */
async function drive(
  mode: "text" | "json",
  messages: string[],
  initialMessage: string | undefined,
  script: {
    prompts: Array<{ input: string; last: AssistantMessage }>;
    header?: Json;
    events?: Json[];
  },
): Promise<{ exit: number; stdout: string; stderr: string }> {
  const out: string[] = [];
  const err: string[] = [];
  const origOut = process.stdout.write.bind(process.stdout);
  const origErr = process.stderr.write.bind(process.stderr);
  process.stdout.write = ((chunk: any, enc?: any, cb?: any) => {
    out.push(String(chunk));
    if (typeof enc === "function") enc();
    else if (typeof cb === "function") cb();
    return true;
  }) as any;
  process.stderr.write = ((chunk: any, enc?: any, cb?: any) => {
    err.push(String(chunk));
    if (typeof enc === "function") enc();
    else if (typeof cb === "function") cb();
    return true;
  }) as any;
  takeOverStdout(); // captures rawStdoutWrite -> our capture fn

  let callNo = 0;
  const session: any = {
    state: { messages: [] as AgentMessage[] },
    async prompt(input: string) {
      const next = script.prompts[callNo++];
      if (!next) throw new Error(`unexpected prompt: ${input}`);
      session.state.messages = [next.last];
      // Emit an event subscribers would see (JSON mode streams these).
      for (const ev of script.events ?? []) {
        for (const sub of subs) sub(ev);
      }
    },
    sessionManager: { getHeader: () => script.header },
    async bindExtensions() {},
    subscribe(fn: (e: any) => void) { subs.push(fn); },
    async reload() {},
  };
  const subs: Array<(e: any) => void> = [];
  const runtime: any = {
    session,
    async dispose() {},
    async newSession() { return { cancelled: false }; },
    async fork() { return { cancelled: false, selectedText: "" }; },
    async switchSession() { return { cancelled: false }; },
    setRebindSession() {},
  };

  let exit: number;
  try {
    exit = await runPrintMode(runtime, {
      mode, messages, initialMessage, initialImages: [],
    });
    await flushRawStdout();
  } finally {
    restoreStdout();
    process.stdout.write = origOut;
    process.stderr.write = origErr;
  }
  return { exit, stdout: out.join(""), stderr: err.join("") };
}

const assistant = (content: Json[], stopReason: string, errorMessage?: string): AssistantMessage => ({
  role: "assistant", content, stopReason,
  ...(errorMessage !== undefined ? { errorMessage } : {}),
  api: "anthropic-messages", provider: "anthropic", model: "m",
  usage: EMPTY_USAGE, timestamp: 0,
});
const text = (text: string) => ({ type: "text", text });
const toolUse = (id: string, name: string, args: Json) => ({ type: "toolCall", id, name, arguments: args });

export async function main(): Promise<void> {
  const out: Json = { oracle: "Pi v0.79.0 c5582102", cases: [] as Json[] };

  const cases: Array<{ name: string; mode: "text" | "json"; messages: string[]; initial: string | undefined; script: Parameters<typeof drive>[3] }> = [
    {
      name: "text-single-block",
      mode: "text", messages: [], initial: "say hi",
      script: { prompts: [{ input: "say hi", last: assistant([text("hello")], "stop") }], },
    },
    {
      name: "text-many-blocks",
      mode: "text", messages: [], initial: "go",
      script: { prompts: [{ input: "go", last: assistant([text("one"), text("two")], "stop") }], },
    },
    {
      name: "text-internal-newlines",
      mode: "text", messages: [], initial: "go",
      script: { prompts: [{ input: "go", last: assistant([text("line1\nline2\nline3")], "stop") }], },
    },
    {
      name: "text-multiline-and-many",
      mode: "text", messages: [], initial: "go",
      script: { prompts: [{ input: "go", last: assistant([text("a\nb"), text("\nc")], "stop") }], },
    },
    {
      name: "text-no-text-content",
      mode: "text", messages: [], initial: "go",
      script: { prompts: [{ input: "go", last: assistant([toolUse("call_1", "bash", { command: "x" })], "toolUse") }], },
    },
    {
      name: "error-with-message",
      mode: "text", messages: [], initial: "go",
      script: { prompts: [{ input: "go", last: assistant([], "error", "connection reset") }] },
    },
    {
      name: "error-no-message",
      mode: "text", messages: [], initial: "go",
      script: { prompts: [{ input: "go", last: assistant([], "error") }] },
    },
    {
      name: "aborted-no-message",
      mode: "text", messages: [], initial: "go",
      script: { prompts: [{ input: "go", last: assistant([], "aborted") }] },
    },
    {
      name: "stop-not-error-has-error-message",
      mode: "text", messages: [], initial: "go",
      script: { prompts: [{ input: "go", last: assistant([text("ok")], "stop", "ignored") }] },
    },
    {
      name: "json-emits-header-and-events",
      mode: "json", messages: [], initial: "go",
      script: {
        prompts: [{ input: "go", last: assistant([text("done")], "stop") }],
        header: { type: "session", id: "sid", version: 1, cwd: "/cwd" },
        events: [
          { type: "message_start", timestamp: 0 },
          { type: "message_update", timestamp: 0 },
          { type: "message_end", timestamp: 0 },
        ],
      },
    },
    {
      name: "json-no-header",
      mode: "json", messages: [], initial: "go",
      script: { prompts: [{ input: "go", last: assistant([text("done")], "stop") }], events: [{ type: "message_end", timestamp: 0 }] },
    },
    {
      name: "multiple-messages-including-initial",
      mode: "text", messages: ["second?", "third?"], initial: "first",
      script: {
        prompts: [
          { input: "first", last: assistant([text("one")], "stop") },
          { input: "second?", last: assistant([text("two")], "stop") },
          { input: "third?", last: assistant([text("three")], "stop") },
        ],
      },
    },
    {
      name: "text-final-message-wins",
      mode: "text", messages: ["second?"], initial: "first",
      script: {
        prompts: [
          { input: "first", last: assistant([text("one")], "stop") },
          { input: "second?", last: assistant([text("two")], "error", "boom") },
        ],
      },
    },
  ];

  for (const c of cases) {
    const r = await drive(c.mode, c.messages, c.initial, c.script);
    out.cases.push({
      name: c.name, mode: c.mode, messages: c.messages, initial: c.initial,
      // Scripted final assistant messages (content/stopReason/errorMessage) so
      // the Rust differential can reproduce the exact session state.
      assistant: c.script.prompts.map((p) => ({
        input: p.input, content: p.last.content, stopReason: p.last.stopReason,
        ...(p.last.errorMessage !== undefined ? { errorMessage: p.last.errorMessage } : {}),
      })),
      header: c.script.header ?? null,
      events: c.script.events ?? [],
      ...r,
    });
  }
  writeFileSync(process.argv[2]!, JSON.stringify(out, null, 2) + "\n");
  console.log(`wrote ${process.argv[2]}`);
}
main().catch((e) => { throw e; });
