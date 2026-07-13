// PLAN 9.2: derive base ExtensionContext snapshots/actions from Pi's real runner.
import { createHarness } from "../../ref/pi/packages/coding-agent/test/suite/harness.ts";

async function main(): Promise<void> {
  const harness = await createHarness({ systemPrompt: "context oracle prompt" });
  try {
    let shutdowns = 0;
    const noUi = {
      select: async () => undefined, confirm: async () => false,
      input: async () => undefined, notify: () => {},
    } as any;
    await harness.session.bindExtensions({
      mode: "tui", uiContext: noUi,
      shutdownHandler: () => { shutdowns++; },
    });
    const startupModel = harness.getModel();
    harness.sessionManager.appendModelChange(startupModel.provider, startupModel.id);
    harness.sessionManager.appendThinkingLevelChange("off");
    const runner = harness.session.extensionRunner;
    const ctx = runner.createCommandContext();
    const model = ctx.model!;
    const found = ctx.modelRegistry.find(model.provider, model.id);
    ctx.shutdown();
    const snapshot = {
      mode: ctx.mode,
      hasUI: ctx.hasUI,
      cwd: "{CWD}",
      trusted: ctx.isProjectTrusted(),
      idle: ctx.isIdle(),
      pending: ctx.hasPendingMessages(),
      hasSignal: ctx.signal !== undefined,
      model: { provider: model.provider, id: model.id },
      session: {
        persisted: ctx.sessionManager.isPersisted(),
        cwd: "{CWD}",
        entries: ctx.sessionManager.getEntries().length,
        branch: ctx.sessionManager.getBranch().length,
      },
      registryFound: found ? { provider: found.provider, id: found.id } : null,
      systemPromptHasCwd: ctx.getSystemPrompt().includes(`Current working directory: ${ctx.cwd}`),
      systemPromptOptionsCwd: ctx.getSystemPromptOptions().cwd === ctx.cwd,
      usage: ctx.getContextUsage() ?? null,
      waitForIdle: typeof ctx.waitForIdle === "function",
      shutdowns,
    };
    runner.invalidate();
    let stale = "";
    try { ctx.isIdle(); } catch (error) { stale = error instanceof Error ? error.message : String(error); }
    process.stdout.write(`${JSON.stringify({ snapshot, stale }, null, "\t")}\n`);
  } finally {
    harness.cleanup();
  }
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
