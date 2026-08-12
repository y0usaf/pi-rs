// PLAN 9.3: Pi-generated oracle for the provider-failure, abort, and reload
// paths of the extension event pipeline. Drives Pi's real AgentSession through
// extension factories (the same public pi.on surface pi-rs uses) and records
// the deterministic extension event trace (agent/turn/message lifecycle,
// input transform, context fold, resources, session lifecycle).
//
// Provider HTTP hooks (before_provider_request / after_provider_response) are
// intentionally excluded here: they only fire through the real HTTP transport
// and are already pinned by the main extension-event oracle (fold) and the
// real_product_seams differential over a real SSE stub. The three paths below
// instead capture the agent-loop event ordering and final session state that
// the interactive runtime reproduces through its scripted streamFn seam.
import { createHarness } from "../../ref/pi/packages/coding-agent/test/suite/harness.ts";
import { fauxAssistantMessage } from "@earendil-works/pi-ai";

type Collector = (line: string) => void;

function traceExtension(prefix: string, collect: Collector) {
	return (pi: any) => {
		for (const type of [
			"input",
			"before_agent_start",
			"agent_start",
			"agent_end",
			"turn_start",
			"turn_end",
			"message_start",
			"message_update",
			"message_end",
			"context",
			"session_start",
			"session_shutdown",
			"resources_discover",
		]) {
			pi.on(type, (event: any) => {
				let line = `${prefix}:${event.type}`;
				if (event.type === "agent_start" || event.type === "agent_end") {
					line += `:${event.messages?.length ?? 0}`;
					if (event.type === "agent_end") line += `:${event.willRetry ?? false}`;
				} else if (event.type === "turn_start" || event.type === "turn_end") {
					line += `:${event.turnIndex}`;
				} else if (
					event.type === "message_start" ||
					event.type === "message_end" ||
					event.type === "message_update"
				) {
					line += `:${event.message?.role ?? "-"}`;
					if (event.type === "message_end" && event.message?.role === "assistant") {
						line += `:${event.message.stopReason ?? ""}:${event.message.errorMessage ?? ""}`;
					}
				} else if (event.type === "resources_discover") {
					line += `:${event.reason}`;
				} else if (event.type === "session_start" || event.type === "session_shutdown") {
					line += `:${event.reason}`;
				}
				collect(line);
			});
		}
	};
}

// Pi's own test suite collapses consecutive duplicate update events
// (normalizeEventOrder) because streaming delta counts are timing-dependent.
function collapseLines(lines: string[]): string[] {
	const out: string[] = [];
	for (const line of lines) {
		if (out[out.length - 1] !== line) out.push(line);
	}
	return out;
}

async function seamProviderFailure(): Promise<unknown> {
	const trace: string[] = [];
	const harness = await createHarness({
		settings: { retry: { enabled: true, maxRetries: 2, baseDelayMs: 1 } },
		extensionFactories: [traceExtension("ext", (l) => trace.push(l))],
	});
	try {
		harness.setResponses([
			fauxAssistantMessage("", { stopReason: "error", errorMessage: "overloaded_error" }),
			fauxAssistantMessage("recovered"),
		]);
		await harness.session.prompt("boom");
		return {
			trace: collapseLines(trace),
			callCount: harness.faux.state.callCount,
			messages: harness.session.messages.map((m: any) => ({
				role: m.role,
				stopReason: m.stopReason,
				errorMessage: m.errorMessage,
			})),
		};
	} finally {
		harness.cleanup();
	}
}

async function seamAbort(): Promise<unknown> {
	const trace: string[] = [];
	const harness = await createHarness({
		extensionFactories: [traceExtension("ext", (l) => trace.push(l))],
		models: [{ id: "slow", name: "Slow", tokensPerSecond: 50 }],
	});
	try {
		harness.setResponses([fauxAssistantMessage("x".repeat(10_000))]);
		const sawUpdate = new Promise<void>((resolve) => {
			harness.session.subscribe((event: any) => {
				if (event.type === "message_update") resolve();
			});
		});
		const promptPromise = harness.session.prompt("hi").catch(() => {});
		await sawUpdate;
		await harness.session.abort();
		await promptPromise;
		return {
			trace: collapseLines(trace),
			messages: harness.session.messages.map((m: any) => ({
				role: m.role,
				stopReason: m.stopReason,
				errorMessage: m.errorMessage,
			})),
		};
	} finally {
		harness.cleanup();
	}
}

async function seamReload(): Promise<unknown> {
	const trace: string[] = [];
	const harness = await createHarness({
		extensionFactories: [traceExtension("ext", (l) => trace.push(l))],
	});
	try {
		await harness.session.bindExtensions({ mode: "tui", uiContext: { notify() {} } as any });
		await harness.session.reload();
		return { trace };
	} finally {
		harness.cleanup();
	}
}

async function main() {
	const result = {
		providerFailure: await seamProviderFailure(),
		abort: await seamAbort(),
		reload: await seamReload(),
	};
	process.stdout.write(JSON.stringify(result, null, "\t") + "\n");
}

main().catch((error) => {
	console.error(error);
	process.exitCode = 1;
});
