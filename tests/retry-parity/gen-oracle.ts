// PLAN 7.10: generate the retry policy oracle from Pi's real AgentSession.
// Classification invokes the private policy only to characterize observable
// retry decisions; run cases drive public prompt/subscribe/abortRetry and record
// stable event, attempt, context-removal, and final-state fields.
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

process.env.PI_CODING_AGENT_DIR = mkdtempSync(join(tmpdir(), "pi-rs-retry-oracle-agentdir-"));
let fauxAssistantMessage: any;
let createHarness: any;

type Json = any;
const spec = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Json;

function stableMessage(message: Json): Json {
  const content = Array.isArray(message?.content) ? message.content : [];
  return {
    role: message?.role,
    text: content.filter((block: Json) => block.type === "text").map((block: Json) => block.text ?? "").join(""),
    ...(message?.stopReason !== undefined ? { stopReason: message.stopReason } : {}),
    ...(message?.errorMessage !== undefined ? { errorMessage: message.errorMessage } : {}),
  };
}

function stableEvent(event: Json): Json | undefined {
  if (event.type === "agent_end") return { type: event.type, willRetry: event.willRetry ?? false };
  if (event.type === "auto_retry_start") {
    return {
      type: event.type, attempt: event.attempt, maxAttempts: event.maxAttempts,
      delayMs: event.delayMs, errorMessage: event.errorMessage,
    };
  }
  if (event.type === "auto_retry_end") {
    return {
      type: event.type, success: event.success, attempt: event.attempt,
      ...(event.finalError !== undefined ? { finalError: event.finalError } : {}),
    };
  }
  if (event.type === "message_end" && event.message?.role === "assistant") {
    return { type: event.type, ...stableMessage(event.message) };
  }
  return undefined;
}

async function runCase(caseSpec: Json): Promise<Json> {
  const modelDefinition = {
    id: spec.model.id, name: spec.model.name, reasoning: false,
    input: ["text"], cost: spec.model.cost,
    contextWindow: spec.model.contextWindow, maxTokens: spec.model.maxTokens,
  };
  const harness = await createHarness({
    models: [modelDefinition],
    settings: caseSpec.mode === "run" ? { retry: caseSpec.settings } : undefined,
  });
  try {
    if (caseSpec.mode === "classify") {
      const message = fauxAssistantMessage("", {
        stopReason: caseSpec.message.stopReason,
        errorMessage: caseSpec.message.errorMessage,
      });
      const retryable = (harness.session as any)._isRetryableError(message);
      return { retryable };
    }

    const contexts: Json[] = [];
    const responses: any[] = caseSpec.turns.map((turn: Json) => (context: Json) => {
      contexts.push(context.messages.map(stableMessage));
      return fauxAssistantMessage(turn.text ?? "", {
        stopReason: turn.stopReason ?? "stop",
        errorMessage: turn.errorMessage,
        timestamp: 0,
      });
    });
    harness.setResponses(responses);
    if (caseSpec.cancelAttempt !== undefined) {
      let cancelled = false;
      harness.session.subscribe((event) => {
        if (event.type === "auto_retry_start" && event.attempt === caseSpec.cancelAttempt && !cancelled) {
          cancelled = true;
          setImmediate(() => harness.session.abortRetry());
        }
      });
    }
    if (caseSpec.queueOnRetry) {
      let queued = false;
      harness.session.subscribe((event) => {
        if (event.type === "agent_end" && event.willRetry && !queued) {
          queued = true;
          void harness.session.followUp(caseSpec.queueOnRetry);
        }
      });
    }
    await harness.session.prompt(caseSpec.prompt ?? "test");
    return {
      events: harness.events.map(stableEvent).filter((event) => event !== undefined),
      callCount: harness.faux.state.callCount,
      contexts,
      messages: harness.session.messages.map(stableMessage),
    };
  } finally {
    harness.cleanup();
  }
}

async function main(): Promise<void> {
  ({ fauxAssistantMessage } = await import("../../ref/pi/packages/ai/src/providers/faux.ts"));
  ({ createHarness } = await import("../../ref/pi/packages/coding-agent/test/suite/harness.ts"));
  const cases = [];
  for (const caseSpec of spec.cases) cases.push({ name: caseSpec.name, ...(await runCase(caseSpec)) });
  process.stdout.write(`${JSON.stringify({ cases }, null, "\t")}\n`);
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
