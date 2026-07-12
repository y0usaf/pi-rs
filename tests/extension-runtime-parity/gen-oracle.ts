// PLAN 9.1: generate the loader/runtime oracle from Pi's real resource loader,
// ExtensionRunner, AgentSession, and faux provider request path.
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, sep } from "node:path";

import { fauxAssistantMessage } from "../../ref/pi/packages/ai/src/providers/faux.ts";
import { DefaultResourceLoader } from "../../ref/pi/packages/coding-agent/src/core/resource-loader.ts";
import { createHarness } from "../../ref/pi/packages/coding-agent/test/suite/harness.ts";

type Json = any;

const root = mkdtempSync(join(tmpdir(), "pi-rs-extension-runtime-oracle-"));
const cwd = join(root, "project");
const agentDir = join(root, "agent");
mkdirSync(cwd, { recursive: true });
mkdirSync(agentDir, { recursive: true });

const sources: Array<[string, string]> = [
  ["01-first.ts", `
    export default async function (pi: any) {
      (globalThis as any).__extensionTrace = ["first:start"];
      await Promise.resolve();
      (globalThis as any).__extensionTrace.push("first:end");
      pi.registerTool({ name: "shared", label: "Shared First", description: "first wins", parameters: {type:"object",properties:{},required:[]}, async execute() { return {content:[{type:"text",text:"first"}],details:{owner:"first"}}; } });
      pi.registerTool({ name: "hello", label: "Hello", description: "A simple greeting tool", parameters: {type:"object",properties:{name:{type:"string",description:"Name to greet"}},required:["name"]}, async execute(_id: string, params: any) { return {content:[{type:"text",text:\`Hello, \${params.name}!\`}],details:{greeted:params.name}}; } });
      pi.registerCommand("dup", { description: "first dup", handler: async () => "first-command" });
      pi.registerCommand("trace", { description: "trace", handler: async () => (globalThis as any).__extensionTrace });
      pi.registerFlag("plan", { description: "Plan mode", type: "boolean", default: false });
      pi.registerFlag("profile", { description: "Profile name", type: "string", default: "safe" });
      pi.registerCommand("flag-values", { handler: async () => ({plan:pi.getFlag("plan"),profile:pi.getFlag("profile"),missing:pi.getFlag("missing")}) });
      pi.on("tool_call", async () => { (globalThis as any).__extensionTrace.push("hook:first"); return {tag:"first"}; });
    }
  `],
  ["02-bad.ts", `
    export default async function (pi: any) {
      (globalThis as any).__extensionTrace.push("bad:start");
      pi.registerTool({ name: "ghost", label: "Ghost", description: "must roll back", parameters: {}, async execute() { return {}; } });
      pi.registerCommand("ghost", { handler: async () => "ghost" });
      pi.on("tool_call", async () => { (globalThis as any).__extensionTrace.push("hook:ghost"); });
      await Promise.resolve();
      throw new Error("broken init");
    }
  `],
  ["03-second.ts", `
    export default async function (pi: any) {
      (globalThis as any).__extensionTrace.push("second:start");
      await new Promise((resolve) => setTimeout(resolve, 1));
      (globalThis as any).__extensionTrace.push("second:end");
      pi.registerTool({ name: "shared", label: "Shared Second", description: "loses", parameters: {type:"object",properties:{},required:[]}, async execute() { return {content:[{type:"text",text:"second"}],details:{owner:"second"}}; } });
      pi.registerCommand("dup", { description: "second dup", handler: async () => "second-command" });
      pi.registerFlag("plan", { description: "Conflicting plan", type: "boolean", default: true });
      pi.registerFlag("second-only", { type: "string" });
      pi.on("tool_call", async () => { (globalThis as any).__extensionTrace.push("hook:second"); return {tag:"second"}; });
    }
  `],
  ["04-block.ts", `
    export default function (pi: any) {
      (globalThis as any).__extensionTrace.push("block:loaded");
      pi.on("tool_call", async () => { (globalThis as any).__extensionTrace.push("hook:block"); return {block:true,reason:"blocked"}; });
      pi.on("tool_call", async () => { (globalThis as any).__extensionTrace.push("hook:after-block"); return {tag:"after"}; });
    }
  `],
];

const paths: string[] = [];
for (const [name, source] of sources) {
  const path = join(root, name);
  writeFileSync(path, source);
  paths.push(path);
}

function stablePath(path: string): string {
  return basename(path).replace(/\.[^.]+$/, "");
}

async function main(): Promise<void> {
  const loader = new DefaultResourceLoader({
    cwd,
    agentDir,
    additionalExtensionPaths: paths,
    noSkills: true,
    noPromptTemplates: true,
    noThemes: true,
    noContextFiles: true,
  });
  await loader.reload();
  const loaded = loader.getExtensions();
  const harness = await createHarness({ resourceLoader: loader });
  try {
    const runner = harness.session.extensionRunner;
    const capturedRequests: Json[] = [];
    harness.setResponses([
      (context: Json) => {
        capturedRequests.push({
          toolNames: (context.tools ?? []).map((tool: Json) => tool.name),
          extensionTools: (context.tools ?? [])
            .filter((tool: Json) => tool.name === "hello" || tool.name === "shared")
            .map((tool: Json) => ({name: tool.name, description: tool.description, parameters: tool.parameters})),
        });
        return fauxAssistantMessage("done", { timestamp: 0 });
      },
    ]);
    await harness.session.prompt("hello");

    const hello = runner.getToolDefinition("hello")!;
    const helloResult = await hello.execute("call-1", {name:"Ada"}, new AbortController().signal, undefined, runner.createContext());
    const commandResults = [];
    for (const name of ["dup:1", "dup:2"]) {
      const command = runner.getCommand(name)!;
      commandResults.push({name, result: await command.handler("", runner.createCommandContext())});
    }
    const hookResult = await runner.emitToolCall({type:"tool_call", toolCallId:"call-2", toolName:"bash", input:{command:"sudo true"}});
    const trace = await runner.getCommand("trace")!.handler("", runner.createCommandContext());
    const flagValues = await runner.getCommand("flag-values")!.handler("", runner.createCommandContext());

    const output = {
      loaded: loaded.extensions.map((extension: Json) => stablePath(extension.path)),
      errors: loaded.errors.map((error: Json) => ({path: stablePath(error.path), error: error.error.replaceAll(`${root}${sep}`, "").replaceAll(".ts", "")})),
      tools: runner.getAllRegisteredTools().map((tool: Json) => ({name: tool.definition.name, source: stablePath(tool.sourceInfo.path)})),
      commands: runner.getRegisteredCommands().map((command: Json) => ({name: command.name, invocationName: command.invocationName, source: stablePath(command.sourceInfo.path), description: command.description ?? null})),
      flags: Array.from(runner.getFlags().values()).map((flag: Json) => ({name: flag.name, source: stablePath(flag.extensionPath), description: flag.description ?? null, type: flag.type, default: flag.default ?? null})),
      commandResults,
      flagValues,
      helloResult,
      hookResult,
      trace,
      capturedRequests,
    };
    process.stdout.write(`${JSON.stringify(output, null, "\t")}\n`);
  } finally {
    harness.cleanup();
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
