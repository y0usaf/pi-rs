import type { ToolResultMessage } from "@earendil-works/pi-ai";
import { DEBOUNCE_MS, HYSTERESIS_MAX_AGE_MS, HYSTERESIS_MIN_RAW_CHARS, HYSTERESIS_MIN_TOOL_CALLS, HYSTERESIS_RECHECK_MS } from "./constants.js";
import type { CapturedBatch, PendingToolCallRecord, ToolCallRecord } from "./types.js";
import { isRecord, textFromContent } from "./utils.js";

function assistantToolArgs(message: unknown): Map<string, unknown> {
	const out = new Map<string, unknown>();
	if (!isRecord(message) || message.role !== "assistant" || !Array.isArray(message.content)) return out;
	for (const part of message.content) {
		if (!isRecord(part) || part.type !== "toolCall" || typeof part.id !== "string") continue;
		out.set(part.id, part.arguments);
	}
	return out;
}

export function captureBatch(turnIndex: number, message: unknown, toolResults: ToolResultMessage[] | undefined, indexed: Map<string, ToolCallRecord>): CapturedBatch | undefined {
	if (!Array.isArray(toolResults) || toolResults.length === 0) return undefined;
	const argsById = assistantToolArgs(message);
	const toolCalls: PendingToolCallRecord[] = [];

	for (const result of toolResults) {
		if (!result?.toolCallId || indexed.has(result.toolCallId)) continue;
		const resultText = textFromContent(result.content);
		if (resultText.trim().length === 0) continue;
		toolCalls.push({
			toolCallId: result.toolCallId,
			toolName: result.toolName,
			args: argsById.get(result.toolCallId),
			resultText,
			isError: result.isError,
			turnIndex,
			timestamp: result.timestamp ?? Date.now(),
		});
	}

	if (toolCalls.length === 0) return undefined;
	return {
		turnIndex,
		toolCalls,
		rawChars: toolCalls.reduce((sum, tool) => sum + tool.resultText.length, 0),
		capturedAt: Date.now(),
	};
}

function pendingTotals(pendingBatches: CapturedBatch[]): { toolCalls: number; rawChars: number } {
	let toolCalls = 0;
	let rawChars = 0;
	for (const batch of pendingBatches) {
		toolCalls += batch.toolCalls.length;
		rawChars += batch.rawChars;
	}
	return { toolCalls, rawChars };
}

export function pendingHysteresis(pendingBatches: CapturedBatch[], now = Date.now()): { ready: boolean; nextDelayMs: number; reason: string } {
	const totals = pendingTotals(pendingBatches);
	const oldestCapturedAt = Math.min(...pendingBatches.map(batch => batch.capturedAt));
	const ageMs = Number.isFinite(oldestCapturedAt) ? Math.max(0, now - oldestCapturedAt) : 0;
	if (totals.toolCalls >= HYSTERESIS_MIN_TOOL_CALLS) return { ready: true, nextDelayMs: DEBOUNCE_MS, reason: "tool-count" };
	if (totals.rawChars >= HYSTERESIS_MIN_RAW_CHARS) return { ready: true, nextDelayMs: DEBOUNCE_MS, reason: "raw-size" };
	if (ageMs >= HYSTERESIS_MAX_AGE_MS) return { ready: true, nextDelayMs: DEBOUNCE_MS, reason: "age" };
	return {
		ready: false,
		nextDelayMs: Math.max(DEBOUNCE_MS, Math.min(HYSTERESIS_RECHECK_MS, HYSTERESIS_MAX_AGE_MS - ageMs)),
		reason: "warming",
	};
}

export function batchFromRecords(records: PendingToolCallRecord[]): CapturedBatch | undefined {
	if (records.length === 0) return undefined;
	return {
		turnIndex: Math.min(...records.map(record => record.turnIndex)),
		toolCalls: records,
		rawChars: records.reduce((sum, record) => sum + record.resultText.length, 0),
		capturedAt: Date.now(),
	};
}
