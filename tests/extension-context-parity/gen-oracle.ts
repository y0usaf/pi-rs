// PLAN 9.2: derive contexts, restrictions, lifecycle action order, and modal
// extension-command delivery from Pi's real runner/AgentSession.
import { createHarness } from "../../ref/pi/packages/coding-agent/test/suite/harness.ts";
import { fauxAssistantMessage } from "@earendil-works/pi-ai";

const staleMessage = "This extension ctx is stale after session replacement or reload. Do not use a captured pi or command ctx after ctx.newSession(), ctx.fork(), ctx.switchSession(), or ctx.reload(). For newSession, fork, and switchSession, move post-replacement work into withSession and use the ctx passed to withSession. For reload, do not use the old ctx after await ctx.reload().";

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function modeMatrix(runner: any): Promise<Array<{ mode: string; hasUI: boolean }>> {
  const modes: Array<{ mode: string; hasUI: boolean }> = [];
  for (const mode of ["print", "json"] as const) {
    runner.setUIContext(undefined, mode);
    const ctx = runner.createContext();
    modes.push({ mode: ctx.mode, hasUI: ctx.hasUI });
  }
  runner.setUIContext({ notify() {} } as any, "rpc");
  const rpc = runner.createContext();
  modes.push({ mode: rpc.mode, hasUI: rpc.hasUI });
  runner.setUIContext({ notify() {} } as any, "tui");
  return modes;
}

async function actionMatrix(runner: any): Promise<any> {
  const trace: string[] = [];
  runner.bindCommandContext({
    waitForIdle: async () => { trace.push("wait"); },
    newSession: async (options: any) => {
      trace.push(`new:${options?.parentSession ?? ""}`);
      return { cancelled: options?.parentSession === "cancel" };
    },
    fork: async (entryId: string, options: any) => {
      trace.push(`fork:${entryId}:${options?.position ?? "before"}`);
      return { cancelled: entryId === "cancel" };
    },
    navigateTree: async (targetId: string, options: any) => {
      trace.push(`tree:${targetId}:${options?.summarize === true}:${options?.label ?? ""}`);
      return { cancelled: targetId === "cancel" };
    },
    switchSession: async (path: string) => {
      trace.push(`switch:${path}`);
      return { cancelled: path === "cancel" };
    },
    reload: async () => { trace.push("reload"); },
  });
  const base = runner.createContext();
  const ctx = runner.createCommandContext();
  await ctx.waitForIdle();
  const outcomes = {
    newSession: await ctx.newSession({ parentSession: "parent.jsonl" }),
    newCancelled: await ctx.newSession({ parentSession: "cancel" }),
    fork: await ctx.fork("entry-1", { position: "at" }),
    forkCancelled: await ctx.fork("cancel"),
    tree: await ctx.navigateTree("entry-2", { summarize: true, label: "kept" }),
    treeCancelled: await ctx.navigateTree("cancel"),
    switchSession: await ctx.switchSession("other.jsonl"),
    switchCancelled: await ctx.switchSession("cancel"),
  };
  await ctx.reload();
  return {
    restrictions: {
      baseNewSession: typeof (base as any).newSession,
      baseFork: typeof (base as any).fork,
      baseTree: typeof (base as any).navigateTree,
      baseSwitch: typeof (base as any).switchSession,
      baseReload: typeof (base as any).reload,
      commandNewSession: typeof ctx.newSession,
      commandFork: typeof ctx.fork,
      commandTree: typeof ctx.navigateTree,
      commandSwitch: typeof ctx.switchSession,
      commandReload: typeof ctx.reload,
    },
    trace,
    outcomes,
  };
}

async function replacementOrder(): Promise<any> {
  const oldHarness = await createHarness();
  const freshHarness = await createHarness();
  try {
    const noUi = { select: async () => undefined, confirm: async () => false, input: async () => undefined, notify: () => {} } as any;
    await oldHarness.session.bindExtensions({ mode: "tui", uiContext: noUi });
    await freshHarness.session.bindExtensions({ mode: "tui", uiContext: noUi });
    const oldRunner = oldHarness.session.extensionRunner;
    const freshRunner = freshHarness.session.extensionRunner;
    const trace: string[] = [];
    oldRunner.bindCommandContext({
      waitForIdle: async () => {},
      newSession: async (options: any) => {
        trace.push("shutdown");
        oldRunner.invalidate();
        trace.push("rebind");
        if (options?.withSession) await options.withSession(freshRunner.createCommandContext());
        trace.push("action-return");
        return { cancelled: false };
      },
      fork: async () => ({ cancelled: false }),
      navigateTree: async () => ({ cancelled: false }),
      switchSession: async () => ({ cancelled: false }),
      reload: async () => {},
    });
    const old = oldRunner.createCommandContext();
    const result = await old.newSession({
      withSession: async (fresh: any) => {
        trace.push("withSession");
        trace.push(`fresh:${fresh.mode}:${fresh.isIdle()}`);
        try { old.isIdle(); } catch (error) { trace.push(`old-stale:${errorText(error) === staleMessage}`); }
      },
    });
    let stale = "";
    try { old.isIdle(); } catch (error) { stale = errorText(error); }
    return { trace, result, stale };
  } finally {
    oldHarness.cleanup();
    freshHarness.cleanup();
  }
}

async function reloadOrder(): Promise<any> {
  const harness = await createHarness();
  try {
    await harness.session.bindExtensions({ mode: "print" });
    const runner = harness.session.extensionRunner;
    const trace: string[] = [];
    runner.bindCommandContext({
      waitForIdle: async () => {}, newSession: async () => ({ cancelled: false }),
      fork: async () => ({ cancelled: false }), navigateTree: async () => ({ cancelled: false }),
      switchSession: async () => ({ cancelled: false }),
      reload: async () => { trace.push("shutdown"); runner.invalidate(); trace.push("reloaded"); },
    });
    const ctx = runner.createCommandContext();
    await ctx.reload();
    let stale = "";
    try { ctx.getSystemPrompt(); } catch (error) { stale = errorText(error); }
    return { trace, stale };
  } finally {
    harness.cleanup();
  }
}

async function waitCancellation(): Promise<any> {
  const harness = await createHarness();
  try {
    const runner = harness.session.extensionRunner as any;
    runner.bindCore(
      {} as any,
      {
        getModel: () => harness.getModel(),
        isIdle: () => !harness.session.agent.isStreaming,
        isProjectTrusted: () => true,
        getSignal: () => harness.session.agent.signal,
        abort: () => harness.session.agent.abort(),
        hasPendingMessages: () => false,
        shutdown: () => {},
        getContextUsage: () => undefined,
        compact: () => {},
        getSystemPrompt: () => "",
      },
    );
    runner.bindCommandContext({
      waitForIdle: () => harness.session.agent.waitForIdle(),
      newSession: async () => ({ cancelled: false }),
      fork: async () => ({ cancelled: false }),
      navigateTree: async () => ({ cancelled: false }),
      switchSession: async () => ({ cancelled: false }),
      reload: async () => {},
    });
    const ctx = runner.createCommandContext();
    const promptPromise = harness.session.prompt("hello")
      .then(() => "prompt-resolved")
      .catch((e: unknown) => "prompt-error:" + (e instanceof Error ? e.message : String(e)));
    await new Promise((r) => setTimeout(r, 20));
    const streaming = harness.session.agent.isStreaming;
    const wait = ctx.waitForIdle().then(
      () => "resolved",
      (e: unknown) => "threw:" + (e instanceof Error ? e.message : String(e)),
    );
    harness.session.agent.abort();
    const waitOutcome = await Promise.race([
      wait,
      new Promise((r) => setTimeout(() => r("pending"), 500)),
    ]);
    const promptOutcome = await promptPromise;
    return { streaming, waitOutcome, promptOutcome };
  } finally {
    harness.cleanup();
  }
}

// Plan 9.2 JSON/RPC delivery: Pi's `session.prompt()` (which print/json/rpc
// modes all drive) routes leading-`/` messages to the registered extension
// command handler with a live command context, and the message is NOT sent to
// the model (provider response stays pending, session messages stay empty).
// print/json modes bind no UI context (hasUI false); rpc-mode.ts binds a real
// createExtensionUIContext (hasUI true + ui.notify callable).
async function delivery(): Promise<any> {
  const result: Record<string, any> = {};
  for (const mode of ["print", "json"] as const) {
    const observed: any[] = [];
    const harness = await createHarness({
      systemPrompt: "delivery oracle prompt",
      extensionFactories: [(pi) => {
        pi.registerCommand("ctx-deliver", {
          description: "deliver", handler: (args: unknown, ctx: any) => {
            observed.push({
              args, mode: ctx.mode, hasUI: ctx.hasUI,
              idle: ctx.isIdle(), trusted: ctx.isProjectTrusted(),
              hasWait: typeof ctx.waitForIdle === "function",
              hasNew: typeof ctx.newSession === "function",
              hasFork: typeof ctx.fork === "function",
              hasTree: typeof ctx.navigateTree === "function",
              hasSwitch: typeof ctx.switchSession === "function",
              hasReload: typeof ctx.reload === "function",
            });
          },
        });
      }],
    });
    try {
      await harness.session.bindExtensions({ mode, commandContextActions: {
        waitForIdle: () => harness.session.agent.waitForIdle(),
        newSession: async () => ({ cancelled: false }),
        fork: async () => ({ cancelled: false }),
        navigateTree: async () => ({ cancelled: false }),
        switchSession: async () => ({ cancelled: false }),
        reload: async () => {},
      } });
      harness.setResponses([fauxAssistantMessage("should stay queued")]);
      await harness.session.prompt(`/ctx-deliver hello ${mode}`);
      result[mode] = {
        observed: JSON.parse(JSON.stringify(observed)),
        pending: harness.getPendingResponseCount(),
        messages: harness.session.messages.length,
      };
    } finally { harness.cleanup(); }
  }
  // RPC mode binds a real UI context (rpc-mode.ts createExtensionUIContext),
  // so ctx.hasUI is true and ctx.ui.notify is callable.
  const rpcObserved: any[] = [];
  const rpcHarness = await createHarness({
    systemPrompt: "delivery oracle prompt",
    extensionFactories: [(pi) => {
      pi.registerCommand("ctx-deliver", {
        description: "deliver", handler: (args: unknown, ctx: any) => {
          rpcObserved.push({
            args, mode: ctx.mode, hasUI: ctx.hasUI,
            hasUiNotify: !!ctx.ui && typeof ctx.ui.notify === "function",
            idle: ctx.isIdle(), trusted: ctx.isProjectTrusted(),
            hasWait: typeof ctx.waitForIdle === "function",
            hasNew: typeof ctx.newSession === "function",
            hasFork: typeof ctx.fork === "function",
            hasTree: typeof ctx.navigateTree === "function",
            hasSwitch: typeof ctx.switchSession === "function",
            hasReload: typeof ctx.reload === "function",
          });
        },
      });
    }],
  });
  try {
    await rpcHarness.session.bindExtensions({ mode: "rpc", uiContext: { notify() {} } as any, commandContextActions: {
      waitForIdle: () => rpcHarness.session.agent.waitForIdle(),
      newSession: async () => ({ cancelled: false }),
      fork: async () => ({ cancelled: false }),
      navigateTree: async () => ({ cancelled: false }),
      switchSession: async () => ({ cancelled: false }),
      reload: async () => {},
    } });
    rpcHarness.setResponses([fauxAssistantMessage("should stay queued")]);
    await rpcHarness.session.prompt("/ctx-deliver hello rpc");
    result["rpc"] = {
      observed: JSON.parse(JSON.stringify(rpcObserved)),
      pending: rpcHarness.getPendingResponseCount(),
      messages: rpcHarness.session.messages.length,
    };
  } finally { rpcHarness.cleanup(); }
  return result;
}

async function main(): Promise<void> {
  const harness = await createHarness({ systemPrompt: "context oracle prompt" });
  try {
    let shutdowns = 0;
    const noUi = { select: async () => undefined, confirm: async () => false, input: async () => undefined, notify: () => {} } as any;
    await harness.session.bindExtensions({ mode: "tui", uiContext: noUi, shutdownHandler: () => { shutdowns++; } });
    const startupModel = harness.getModel();
    harness.sessionManager.appendModelChange(startupModel.provider, startupModel.id);
    harness.sessionManager.appendThinkingLevelChange("off");
    const runner = harness.session.extensionRunner;
    const modes = await modeMatrix(runner);
    const ctx = runner.createCommandContext();
    const model = ctx.model!;
    const found = ctx.modelRegistry.find(model.provider, model.id);
    ctx.shutdown();
    const snapshot = {
      mode: ctx.mode, hasUI: ctx.hasUI, cwd: "{CWD}", trusted: ctx.isProjectTrusted(),
      idle: ctx.isIdle(), pending: ctx.hasPendingMessages(), hasSignal: ctx.signal !== undefined,
      model: { provider: model.provider, id: model.id },
      session: { persisted: ctx.sessionManager.isPersisted(), cwd: "{CWD}", entries: ctx.sessionManager.getEntries().length, branch: ctx.sessionManager.getBranch().length },
      registryFound: found ? { provider: found.provider, id: found.id } : null,
      systemPromptHasCwd: ctx.getSystemPrompt().includes(`Current working directory: ${ctx.cwd}`),
      systemPromptOptionsCwd: ctx.getSystemPromptOptions().cwd === ctx.cwd,
      usage: ctx.getContextUsage() ?? null, waitForIdle: typeof ctx.waitForIdle === "function", shutdowns,
    };
    runner.invalidate();
    let stale = "";
    try { ctx.isIdle(); } catch (error) { stale = errorText(error); }
    const actionsHarness = await createHarness();
    let actions: any;
    try {
      await actionsHarness.session.bindExtensions({ mode: "tui", uiContext: noUi });
      actions = await actionMatrix(actionsHarness.session.extensionRunner);
    } finally { actionsHarness.cleanup(); }
    process.stdout.write(`${JSON.stringify({ snapshot, stale, modes, actions, replacement: await replacementOrder(), reload: await reloadOrder(), waitCancellation: await waitCancellation(), delivery: await delivery() }, null, "\t")}\n`);
  } finally { harness.cleanup(); }
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
