import { createHash } from "node:crypto";
import { completeSimple as complete, type Model, type Usage } from "@earendil-works/pi-ai";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import { AUTO_MODEL_CANDIDATES, DECIDER_SYSTEM_PROMPT, MAX_DECIDER_INPUT_CHARS, MAX_DECIDER_TOKENS } from "./constants.js";
import type { DeciderAction, DeciderObject, PendingToolCallRecord } from "./types.js";
import { formatChars, isRecord, safeJson, stableJson, truncateMiddle } from "./utils.js";

function hashObject(value: unknown): string {
	return createHash("sha256").update(stableJson(value)).digest("hex").slice(0, 16);
}

function modelKey(provider: string, modelId: string): string {
	return `${provider}/${modelId}`.toLowerCase();
}

function findAutoCandidate(ctx: ExtensionContext, provider: string, modelId: string): Model<any> | undefined {
	const model = ctx.modelRegistry.find(provider, modelId) as Model<any> | undefined;
	if (!model) return undefined;
	const available = ctx.modelRegistry.getAvailable() as Model<any>[];
	const availableKeys = new Set(available.map(item => modelKey(String(item.provider), String(item.id))));
	return availableKeys.has(modelKey(provider, modelId)) ? model : undefined;
}

function resolveLightweightModel(ctx: ExtensionContext): Model<any> {
	const activeProvider = ctx.model?.provider;
	if (activeProvider) {
		const activeCandidate = AUTO_MODEL_CANDIDATES.find(candidate => candidate.provider === activeProvider);
		if (activeCandidate) {
			const model = findAutoCandidate(ctx, activeCandidate.provider, activeCandidate.modelId);
			if (model) return model;
		}
	}

	for (const candidate of AUTO_MODEL_CANDIDATES) {
		const model = findAutoCandidate(ctx, candidate.provider, candidate.modelId);
		if (model) return model;
	}

	if (ctx.model) return ctx.model as Model<any>;
	throw new Error("No lightweight janitor model is available. Configure OpenAI, Anthropic, Vercel AI Gateway, or select an active Pi model.");
}

function deciderObject(record: PendingToolCallRecord, argsBudget: number, outputBudget: number): DeciderObject {
	const object = {
		id: record.toolCallId,
		kind: "tool_result" as const,
		toolName: record.toolName,
		status: record.isError ? "error" as const : "ok" as const,
		turnIndex: record.turnIndex,
		rawChars: record.resultText.length,
		argsPreview: safeJson(record.args, argsBudget),
		outputPreview: truncateMiddle(record.resultText, outputBudget),
	};
	return { ...object, hash: hashObject(object) };
}

function buildDeciderInput(records: PendingToolCallRecord[]): { input: string; candidates: Map<string, DeciderObject> } {
	let argsBudget = 1_200;
	let outputBudget = 2_000;
	let objects: DeciderObject[] = [];
	let input = "";

	for (let attempt = 0; attempt < 8; attempt += 1) {
		objects = records.map(record => deciderObject(record, argsBudget, outputBudget));
		input = JSON.stringify({
			instruction: "For each tool_result object, choose action=truncate only if its output is safe to replace with a hidden placeholder in future context. Otherwise choose keep.",
			actions: ["truncate", "keep"],
			objects,
		}, null, 2);
		if (input.length <= MAX_DECIDER_INPUT_CHARS) break;
		argsBudget = Math.max(160, Math.floor(argsBudget * 0.55));
		outputBudget = Math.max(240, Math.floor(outputBudget * 0.55));
	}

	if (input.length > MAX_DECIDER_INPUT_CHARS) {
		throw new Error(`Janitor decider input is too large (${formatChars(input.length)}).`);
	}

	return { input, candidates: new Map(objects.map(object => [object.id, object] as const)) };
}

function extractJsonObject(text: string): unknown {
	const cleaned = text.trim().replace(/^```(?:json)?\s*/i, "").replace(/\s*```$/i, "").trim();
	let parseError: unknown;
	try {
		return JSON.parse(cleaned) as unknown;
	} catch (error) {
		parseError = error;
		const start = cleaned.indexOf("{");
		const end = cleaned.lastIndexOf("}");
		if (start >= 0 && end > start) {
			try {
				return JSON.parse(cleaned.slice(start, end + 1)) as unknown;
			} catch (sliceError) {
				parseError = sliceError;
			}
		}
	}
	const detail = parseError instanceof Error && parseError.message ? `: ${parseError.message}` : "";
	throw new Error(`Janitor decider returned invalid JSON${detail}.`);
}

function isDeciderFormatError(error: unknown): boolean {
	if (!(error instanceof Error)) return false;
	return error.message.startsWith("Janitor decider returned invalid JSON")
		|| error.message === "Janitor decider JSON must contain an actions array.";
}

function parseDeciderActions(raw: unknown, candidates: Map<string, DeciderObject>): DeciderAction[] {
	if (!isRecord(raw) || !Array.isArray(raw.actions)) throw new Error("Janitor decider JSON must contain an actions array.");
	const out: DeciderAction[] = [];
	for (const item of raw.actions) {
		if (!isRecord(item) || !isRecord(item.target)) continue;
		const id = typeof item.target.id === "string" ? item.target.id : undefined;
		const hash = typeof item.target.hash === "string" ? item.target.hash : undefined;
		const action = item.action === "truncate" || item.action === "hide" ? "truncate" : item.action === "keep" ? "keep" : undefined;
		if (!id || !hash || !action) continue;
		const candidate = candidates.get(id);
		if (!candidate || candidate.hash !== hash) continue;
		out.push({
			target: { id, hash },
			action,
			reason: typeof item.reason === "string" && item.reason.trim() ? item.reason.trim().slice(0, 160) : action,
		});
	}
	return out;
}

export async function decideRecords(ctx: ExtensionContext, records: PendingToolCallRecord[], signal: AbortSignal): Promise<{ records: PendingToolCallRecord[]; usage?: Usage; modelLabel: string }> {
	const model = resolveLightweightModel(ctx);
	const apiKey = await ctx.modelRegistry.getApiKeyForProvider(model.provider);
	const { input, candidates } = buildDeciderInput(records);
	const response = await complete(
		model,
		{
			systemPrompt: DECIDER_SYSTEM_PROMPT,
			messages: [
				{
					role: "user",
					content: input,
					timestamp: Date.now(),
				},
			],
		},
		{
			apiKey,
			signal,
			maxTokens: MAX_DECIDER_TOKENS,
			temperature: 0,
		},
	);

	const text = response.content
		.filter((part): part is { type: "text"; text: string } => part.type === "text")
		.map(part => part.text)
		.join("\n")
		.trim();
	if (!text) throw new Error("Janitor decider returned no text.");

	let actions: DeciderAction[];
	try {
		actions = parseDeciderActions(extractJsonObject(text), candidates);
	} catch (error) {
		if (!isDeciderFormatError(error)) throw error;
		return { records: [], usage: response.usage, modelLabel: `${model.provider}/${model.id}` };
	}

	const truncateById = new Map(actions.filter(action => action.action === "truncate").map(action => [action.target.id, action] as const));
	return {
		records: records
			.filter(record => truncateById.has(record.toolCallId))
			.map(record => {
				const action = truncateById.get(record.toolCallId)!;
				return { ...record, hash: action.target.hash, janitorReason: action.reason };
			}),
		usage: response.usage,
		modelLabel: `${model.provider}/${model.id}`,
	};
}
