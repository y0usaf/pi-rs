import type { Usage } from "@earendil-works/pi-ai";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";

export interface JanitorSettings {
	enabled: boolean;
}

export interface ToolCallRecord {
	toolCallId: string;
	toolName: string;
	args: unknown;
	resultText: string;
	isError: boolean;
	turnIndex: number;
	timestamp: number;
	summaryId: string;
	hash?: string;
	janitorReason?: string;
}

export interface PendingToolCallRecord extends Omit<ToolCallRecord, "summaryId"> {}

export interface CapturedBatch {
	turnIndex: number;
	toolCalls: PendingToolCallRecord[];
	rawChars: number;
	capturedAt: number;
}

export interface SummaryIndexEntry {
	version: 1;
	summaryId: string;
	createdAt: string;
	reason: string;
	rawChars: number;
	projectedChars: number;
	deciderModel: string;
	usage?: Usage;
	toolCalls: ToolCallRecord[];
}

export interface RestoreIndexEntry {
	version: 1;
	restoreId: string;
	createdAt: string;
	reason: string;
	summaryIds: string[];
}

export interface DeciderObject {
	id: string;
	hash: string;
	kind: "tool_result";
	toolName: string;
	status: "ok" | "error";
	turnIndex: number;
	rawChars: number;
	argsPreview: string;
	outputPreview: string;
}

export interface DeciderAction {
	target: { id: string; hash: string };
	action: "truncate" | "keep";
	reason: string;
}

export type KeybindingsLike = { matches(data: string, action: string): boolean };
export type ThemeLike = ExtensionContext["ui"]["theme"];

export interface UndoRunItem {
	summaryId: string;
	label: string;
	description: string;
}
