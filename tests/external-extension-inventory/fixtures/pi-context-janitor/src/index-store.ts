import { CONTEXT_HIDDEN_TEXT } from "./constants.js";
import type { PendingToolCallRecord, RestoreIndexEntry, SummaryIndexEntry, ToolCallRecord } from "./types.js";
import { isRecord } from "./utils.js";

export function makeSummaryId(): string {
	return `cj-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

export function makeRestoreId(): string {
	return `cj-restore-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

// Do not expose janitor metadata to the main model. Restore metadata lives in hidden custom entries.
export function projectionText(_record: ToolCallRecord): string {
	return CONTEXT_HIDDEN_TEXT;
}

export function entryFromRun(summaryId: string, reason: string, records: PendingToolCallRecord[], result: { usage?: SummaryIndexEntry["usage"]; modelLabel: string }): SummaryIndexEntry {
	const toolCalls = records.map(record => ({ ...record, summaryId }));
	return {
		version: 1,
		summaryId,
		createdAt: new Date().toISOString(),
		reason,
		rawChars: toolCalls.reduce((sum, record) => sum + record.resultText.length, 0),
		projectedChars: toolCalls.reduce((sum, record) => sum + projectionText(record).length, 0),
		deciderModel: result.modelLabel,
		usage: result.usage,
		toolCalls,
	};
}

export function applyIndexEntry(entry: SummaryIndexEntry, index: Map<string, ToolCallRecord>, entries: Map<string, SummaryIndexEntry>): void {
	entries.set(entry.summaryId, entry);

	for (const record of entry.toolCalls) {
		if (!record.toolCallId || typeof record.resultText !== "string") continue;
		index.set(record.toolCallId, record);
	}
}

export function parseIndexEntry(raw: unknown): SummaryIndexEntry | undefined {
	if (!isRecord(raw) || raw.version !== 1 || typeof raw.summaryId !== "string" || !Array.isArray(raw.toolCalls)) return undefined;
	const toolCalls: ToolCallRecord[] = [];
	for (const item of raw.toolCalls) {
		if (!isRecord(item)) continue;
		if (typeof item.toolCallId !== "string" || typeof item.toolName !== "string" || typeof item.resultText !== "string") continue;
		toolCalls.push({
			toolCallId: item.toolCallId,
			toolName: item.toolName,
			args: item.args,
			resultText: item.resultText,
			isError: item.isError === true,
			turnIndex: typeof item.turnIndex === "number" ? item.turnIndex : 0,
			timestamp: typeof item.timestamp === "number" ? item.timestamp : Date.now(),
			summaryId: typeof item.summaryId === "string" ? item.summaryId : raw.summaryId,
			hash: typeof item.hash === "string" ? item.hash : undefined,
			janitorReason: typeof item.janitorReason === "string" ? item.janitorReason : undefined,
		});
	}
	if (toolCalls.length === 0 || toolCalls.some(record => typeof record.hash !== "string")) return undefined;
	return {
		version: 1,
		summaryId: raw.summaryId,
		createdAt: typeof raw.createdAt === "string" ? raw.createdAt : new Date().toISOString(),
		reason: typeof raw.reason === "string" ? raw.reason : "reconstruct",
		rawChars: typeof raw.rawChars === "number" ? raw.rawChars : toolCalls.reduce((sum, record) => sum + record.resultText.length, 0),
		projectedChars: toolCalls.reduce((sum, record) => sum + projectionText(record).length, 0),
		deciderModel: typeof raw.deciderModel === "string" ? raw.deciderModel : "unknown",
		usage: isRecord(raw.usage) ? raw.usage as unknown as SummaryIndexEntry["usage"] : undefined,
		toolCalls,
	};
}

export function parseRestoreEntry(raw: unknown): RestoreIndexEntry | undefined {
	if (!isRecord(raw) || raw.version !== 1 || typeof raw.restoreId !== "string" || !Array.isArray(raw.summaryIds)) return undefined;
	const summaryIds = raw.summaryIds.filter((id): id is string => typeof id === "string" && id.trim().length > 0).map(id => id.trim());
	if (summaryIds.length === 0) return undefined;
	return {
		version: 1,
		restoreId: raw.restoreId,
		createdAt: typeof raw.createdAt === "string" ? raw.createdAt : new Date().toISOString(),
		reason: typeof raw.reason === "string" ? raw.reason : "restore",
		summaryIds: [...new Set(summaryIds)],
	};
}

export function activeSavings(entries: Map<string, SummaryIndexEntry>, restoredSummaryIds: Set<string>): { activeRuns: number; restoredRuns: number; rawChars: number; projectedChars: number; savedChars: number } {
	let activeRuns = 0;
	let restoredRuns = 0;
	let rawChars = 0;
	let projectedChars = 0;
	for (const entry of entries.values()) {
		if (restoredSummaryIds.has(entry.summaryId)) {
			restoredRuns += 1;
			continue;
		}
		activeRuns += 1;
		rawChars += entry.rawChars;
		projectedChars += entry.projectedChars;
	}
	return { activeRuns, restoredRuns, rawChars, projectedChars, savedChars: Math.max(0, rawChars - projectedChars) };
}
