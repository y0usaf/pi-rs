// Regenerates tests/compaction-parity/oracle.json by driving Pi's real
// compaction pipeline (ref/pi @ c5582102 core/compaction/compaction.ts and
// pi-ai utils/overflow.ts) over the scripted session entries in cases.json.
// Modes:
//   prepare  — prepareCompaction(entries, settings): the cut point,
//              messages to summarize, split-turn prefix, previous-summary
//              merge, file ops, and tokensBefore.
//   compact  — prepareCompaction + compact() with an injected scripted
//              streamFn recording every summarization request (system
//              prompt, prompt text, maxTokens, reasoning) and the final
//              summary/details, or the thrown error.
//   tokens   — estimateContextTokens / calculateContextTokens.
//   should   — shouldCompact.
//   overflow — isContextOverflow (pi-ai overflow patterns).
// Undefined/null fields are omitted so the Lua replay (which drops nils)
// compares as parsed JSON. Date.now is pinned; the Lua replay passes the
// same now_ms. Run via scripts/compaction-oracle. Do not edit the oracle
// by hand.
import { readFileSync } from "node:fs";
import type { AssistantMessage } from "../../ref/pi/packages/ai/src/types.ts";
import { isContextOverflow } from "../../ref/pi/packages/ai/src/utils/overflow.ts";
import {
	calculateContextTokens,
	compact,
	estimateContextTokens,
	prepareCompaction,
	shouldCompact,
} from "../../ref/pi/packages/coding-agent/src/core/compaction/compaction.ts";
import type { SessionEntry } from "../../ref/pi/packages/coding-agent/src/core/session-manager.ts";

type Json = any;

const NOW_MS = 1750000000000;
const RealDate = Date;
class FixedDate extends RealDate {
	constructor(...args: ConstructorParameters<typeof RealDate>) {
		if (args.length === 0) {
			super(NOW_MS);
		} else {
			super(...args);
		}
	}
	static now(): number {
		return NOW_MS;
	}
}
(globalThis as { Date: unknown }).Date = FixedDate;

const casesPath = process.argv[2] ?? "tests/compaction-parity/cases.json";
const spec = JSON.parse(readFileSync(casesPath, "utf8")) as Json;

/** Drop undefined/null recursively so both sides compare field-wise. */
function stripNull(value: Json): Json {
	if (Array.isArray(value)) return value.map(stripNull);
	if (value && typeof value === "object") {
		const out: Json = {};
		for (const [key, item] of Object.entries(value)) {
			if (item === undefined || item === null) continue;
			out[key] = stripNull(item);
		}
		return out;
	}
	return value;
}

function fileOpsToJson(fileOps: { read: Set<string>; written: Set<string>; edited: Set<string> }): Json {
	return {
		read: [...fileOps.read].sort(),
		written: [...fileOps.written].sort(),
		edited: [...fileOps.edited].sort(),
	};
}

function preparationToJson(preparation: Json): Json {
	return {
		firstKeptEntryId: preparation.firstKeptEntryId,
		isSplitTurn: preparation.isSplitTurn,
		tokensBefore: preparation.tokensBefore,
		previousSummary: preparation.previousSummary,
		messagesToSummarize: preparation.messagesToSummarize,
		turnPrefixMessages: preparation.turnPrefixMessages,
		fileOps: fileOpsToJson(preparation.fileOps),
	};
}

function scriptedStreamFn(caseSpec: Json, requests: Json[]): Json {
	return async (model: Json, context: Json, options: Json) => {
		const index = requests.length;
		requests.push({
			systemPrompt: context.systemPrompt,
			messages: context.messages,
			maxTokens: options.maxTokens,
			reasoning: options.reasoning,
			apiKey: options.apiKey,
		});
		const scripted = (caseSpec.responses ?? [])[index] ?? { text: "" };
		const message: AssistantMessage = {
			role: "assistant",
			content: scripted.errorMessage ? [] : [{ type: "text", text: scripted.text ?? "" }],
			api: model.api,
			provider: model.provider,
			model: model.id,
			usage: {
				input: 0,
				output: 0,
				cacheRead: 0,
				cacheWrite: 0,
				totalTokens: 0,
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
			},
			stopReason: scripted.errorMessage ? "error" : "stop",
			errorMessage: scripted.errorMessage,
			timestamp: 0,
		};
		return { result: async () => message } as Json;
	};
}

async function runCase(caseSpec: Json): Promise<Json> {
	const mode = caseSpec.mode ?? "prepare";
	const model = spec.models[caseSpec.model ?? "default"];
	const settings = {
		enabled: true,
		reserveTokens: 16384,
		keepRecentTokens: 20000,
		...(caseSpec.settings ?? {}),
	};

	if (mode === "tokens") {
		const out: Json = {};
		if (caseSpec.messages) out.estimate = estimateContextTokens(caseSpec.messages);
		if (caseSpec.usage) out.contextTokens = calculateContextTokens(caseSpec.usage);
		return out;
	}
	if (mode === "should") {
		return {
			shouldCompact: shouldCompact(caseSpec.contextTokens, caseSpec.contextWindow, settings),
		};
	}
	if (mode === "overflow") {
		return {
			overflow: isContextOverflow(caseSpec.message, caseSpec.contextWindow),
		};
	}

	const entries = caseSpec.entries as SessionEntry[];
	const preparation = prepareCompaction(entries, settings);
	if (!preparation) {
		return { prepared: false };
	}
	const result: Json = { prepared: true, preparation: preparationToJson(preparation) };
	if (mode === "compact") {
		const requests: Json[] = [];
		try {
			const compactResult = await compact(
				preparation,
				model,
				caseSpec.apiKey ?? "oracle-key",
				undefined,
				caseSpec.customInstructions,
				undefined,
				caseSpec.thinkingLevel,
				scriptedStreamFn(caseSpec, requests),
			);
			result.result = compactResult;
		} catch (error) {
			result.error = error instanceof Error ? error.message : String(error);
		}
		result.requests = requests;
	}
	return result;
}

async function main() {
	const cases: Json[] = [];
	for (const caseSpec of spec.cases) {
		cases.push({ name: caseSpec.name, ...stripNull(await runCase(caseSpec)) });
	}
	process.stdout.write(`${JSON.stringify({ cases }, null, "\t")}\n`);
}

main().catch((error) => {
	console.error(error);
	process.exitCode = 1;
});
