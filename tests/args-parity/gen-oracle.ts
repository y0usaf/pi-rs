// Peel the pinned Pi args module surface for the pi-rs differential oracle.
// Records parseArgs() results for a corpus of CLI inputs plus printHelp()
// output (with chalk stripped), so pi-rs's Rust args/help can be compared.
//
// Run via scripts/args-oracle. Offline normal checks consume the checked
// oracle; opt-in regeneration drives the pinned Pi source.
import type {
  Args,
  ExtensionFlag,
} from "../../ref/pi/packages/coding-agent/src/cli/args.ts";
import {
  isValidThinkingLevel,
  parseArgs,
  printHelp,
} from "../../ref/pi/packages/coding-agent/src/cli/args.ts";
import { writeFileSync } from "node:fs";

// Strip ANSI/color codes so the help text compares cleanly across termcap.
function stripAnsi(s: string): string {
  // eslint-disable-next-line no-control-regex
  return s.replace(/\u001b\[[0-9;]*m/g, "");
}

type Json = any;

function normalizeArgs(a: Args): Json {
  const out: any = { messages: a.messages, fileArgs: a.fileArgs };
  const scalar: Array<[string, boolean | string | undefined]> = [
    ["provider", a.provider],
    ["model", a.model],
    ["apiKey", a.apiKey],
    ["systemPrompt", a.systemPrompt],
    ["continue", a.continue],
    ["resume", a.resume],
    ["help", a.help],
    ["version", a.version],
    ["mode", a.mode],
    ["name", a.name],
    ["noSession", a.noSession],
    ["session", a.session],
    ["sessionId", a.sessionId],
    ["fork", a.fork],
    ["sessionDir", a.sessionDir],
    ["print", a.print],
    ["export", a.export],
    ["noTools", a.noTools],
    ["noBuiltinTools", a.noBuiltinTools],
    ["noExtensions", a.noExtensions],
    ["noSkills", a.noSkills],
    ["noPromptTemplates", a.noPromptTemplates],
    ["noThemes", a.noThemes],
    ["noContextFiles", a.noContextFiles],
    ["offline", a.offline],
    ["verbose", a.verbose],
    ["projectTrustOverride", a.projectTrustOverride],
    ["listModels", a.listModels],
    ["thinking", a.thinking],
  ];
  for (const [k, v] of scalar) {
    if (v !== undefined) out[k] = v;
  }
  const lists: Array<[string, string[] | undefined]> = [
    ["appendSystemPrompt", a.appendSystemPrompt],
    ["models", a.models],
    ["tools", a.tools],
    ["excludeTools", a.excludeTools],
    ["extensions", a.extensions],
    ["skills", a.skills],
    ["promptTemplates", a.promptTemplates],
    ["themes", a.themes],
  ];
  for (const [k, v] of lists) {
    if (v && v.length > 0) out[k] = v;
  }
  if (a.diagnostics.length > 0) {
    out.diagnostics = a.diagnostics;
  }
  return out;
}

const CORPUS: Array<{ name: string; argv: string[] }> = [
  { name: "help-h", argv: ["-h"] },
  { name: "help-long", argv: ["--help"] },
  { name: "version-v", argv: ["-v"] },
  { name: "version-long", argv: ["--version"] },
  { name: "mode-text", argv: ["--mode", "text"] },
  { name: "mode-json", argv: ["--mode", "json"] },
  { name: "mode-rpc", argv: ["--mode", "rpc"] },
  { name: "mode-invalid", argv: ["--mode", "bogus"] },
  { name: "mode-missing-value", argv: ["--mode"] },
  { name: "empty", argv: [] },
  { name: "single-message", argv: ["hello world"] },
  { name: "multi-message", argv: ["a", "b", "c"] },
  { name: "print-simple", argv: ["--print"] },
  { name: "print-short", argv: ["-p"] },
  { name: "print-with-message", argv: ["-p", "do it"] },
  { name: "print-following-message", argv: ["--print", "do it", "next"] },
  { name: "print-flag-next", argv: ["-p", "--version"] },
  { name: "print-atfile-next", argv: ["-p", "@file"] },
  { name: "provider-model", argv: ["--provider", "anthropic", "--model", "opus"] },
  { name: "model-shorthand", argv: ["--model", "sonnet:high"] },
  { name: "model-provider-prefix", argv: ["--model", "openai/gpt-4o"] },
  { name: "api-key", argv: ["--api-key", "sk-123"] },
  { name: "system-prompt", argv: ["--system-prompt", "be brief"] },
  { name: "append-system-prompt-1", argv: ["--append-system-prompt", "x"] },
  { name: "append-system-prompt-many", argv: ["--append-system-prompt", "x", "--append-system-prompt", "y"] },
  { name: "thinking-off", argv: ["--thinking", "off"] },
  { name: "thinking-high", argv: ["--thinking", "high"] },
  { name: "thinking-invalid", argv: ["--thinking", "ultra"] },
  { name: "thinking-missing-value", argv: ["--thinking"] },
  { name: "continue-c", argv: ["--continue"] },
  { name: "continue-short", argv: ["-c"] },
  { name: "resume-r", argv: ["--resume"] },
  { name: "resume-short", argv: ["-r"] },
  { name: "session", argv: ["--session", "abc123"] },
  { name: "session-id", argv: ["--session-id", "xyz"] },
  { name: "fork", argv: ["--fork", "feed"] },
  { name: "session-dir", argv: ["--session-dir", "/tmp/s"] },
  { name: "no-session", argv: ["--no-session"] },
  { name: "name", argv: ["--name", "my session"] },
  { name: "name-short", argv: ["-n"] },
  { name: "name-missing-value", argv: ["--name"] },
  { name: "models", argv: ["--models", "a,b, c"] },
  { name: "no-tools", argv: ["--no-tools"] },
  { name: "no-tools-short", argv: ["-nt"] },
  { name: "no-builtin-tools", argv: ["--no-builtin-tools"] },
  { name: "no-builtin-tools-short", argv: ["-nbt"] },
  { name: "tools-empty-comma", argv: ["--tools", ","] },
  { name: "tools", argv: ["--tools", "read,bash"] },
  { name: "exclude-tools", argv: ["--exclude-tools", "ask_question"] },
  { name: "extension-1", argv: ["--extension", "one.lua"] },
  { name: "extension-many", argv: ["-e", "one.lua", "--extension", "two.lua"] },
  { name: "no-extensions", argv: ["--no-extensions"] },
  { name: "skill", argv: ["--skill", "s.lua"] },
  { name: "no-skills", argv: ["--no-skills"] },
  { name: "prompt-template", argv: ["--prompt-template", "p.md"] },
  { name: "no-prompt-templates", argv: ["--no-prompt-templates"] },
  { name: "theme", argv: ["--theme", "t.json"] },
  { name: "no-themes", argv: ["--no-themes"] },
  { name: "no-context-files", argv: ["--no-context-files"] },
  { name: "list-models-flag", argv: ["--list-models"] },
  { name: "list-models-pattern", argv: ["--list-models", "opus"] },
  { name: "list-models-next-flag", argv: ["--list-models", "--version"] },
  { name: "verbose", argv: ["--verbose"] },
  { name: "approve-a", argv: ["--approve"] },
  { name: "no-approve-na", argv: ["--no-approve"] },
  { name: "unknown-short", argv: ["-zz"] },
  { name: "unknown-long", argv: ["--nonsense"] },
  { name: "unknown-long-value", argv: ["--nonsense", "value"] },
  { name: "unknown-long-eq", argv: ["--foo=bar"] },
  { name: "atfile-prompt", argv: ["@prompt.md", "@img.png", "color?"] },
  { name: "atfile-empty", argv: ["@"] },
  { name: "all-at-end", argv: ["--print", "m1", "m2", "@f.txt"] },
  { name: "export", argv: ["--export", "out.html"] },
];

const HELP_CASES: Array<{ name: string; flags?: ExtensionFlag[] }> = [
  { name: "no-extension-flags" },
  {
    name: "with-extension-flags",
    flags: [
      { name: "plan", type: "string" as const, description: "Plan mode", extensionPath: "/x/plan.ts" },
      { name: "dry", type: "boolean" as const, extensionPath: "/x/dry.ts" },
    ],
  },
];

export async function main(): Promise<void> {
  const out: Json = {
    oracle: "Pi v0.79.0 c5582102",
    validThinkingLevels: ["off", "minimal", "low", "medium", "high", "xhigh"].map((l) => [
      l,
      isValidThinkingLevel(l),
    ]),
    cases: [] as Json[],
    help: [] as Json[],
  };

  for (const c of CORPUS) {
    const args = parseArgs(c.argv);
    out.cases.push({ name: c.name, argv: c.argv, args: normalizeArgs(args) });
  }

  for (const c of HELP_CASES) {
    // capture printHelp stdout
    let captured = "";
    const orig = process.stdout.write.bind(process.stdout);
    (process.stdout as any).write = function (chunk: any, ...rest: any[]) {
      captured += String(chunk);
      const cb = typeof rest[rest.length - 1] === "function" ? rest[rest.length - 1] : undefined;
      if (cb) cb();
      return true;
    } as any;
    printHelp(c.flags);
    (process.stdout as any).write = orig;
    out.help.push({ name: c.name, text: stripAnsi(captured) });
  }

  writeFileSync(process.argv[2]!, JSON.stringify(out, null, 2) + "\n");
  console.log(`wrote ${process.argv[2]}`);
}
main().catch((e) => { throw e; });
