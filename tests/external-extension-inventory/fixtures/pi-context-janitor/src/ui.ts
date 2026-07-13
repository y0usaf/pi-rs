import type { Component } from "@earendil-works/pi-tui";
import { truncateToWidth } from "@earendil-works/pi-tui";
import type { KeybindingsLike, SummaryIndexEntry, ThemeLike, UndoRunItem } from "./types.js";
import { formatChars, isPiCompactEnabled, isRecord, replaceTabs, shortTimestamp, themeBold, themeFg, truncateMiddle } from "./utils.js";

function summarizeToolNames(records: ReadonlyArray<{ toolName: string }>, maxNames = 5): string {
	const counts = new Map<string, number>();
	for (const record of records) counts.set(record.toolName, (counts.get(record.toolName) ?? 0) + 1);
	const parts = Array.from(counts.entries())
		.sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
		.map(([name, count]) => count > 1 ? `${name}×${count}` : name);
	if (parts.length <= maxNames) return parts.join(", ");
	return `${parts.slice(0, maxNames).join(", ")} +${parts.length - maxNames} more`;
}

export function undoRunItems(entries: Map<string, SummaryIndexEntry>, restoredSummaryIds: Set<string>): UndoRunItem[] {
	return Array.from(entries.values())
		.filter(entry => !restoredSummaryIds.has(entry.summaryId) && entry.rawChars > 0)
		.sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt))
		.map(entry => {
			const saved = Math.max(0, entry.rawChars - entry.projectedChars);
			return {
				summaryId: entry.summaryId,
				label: `${shortTimestamp(entry.createdAt)}  truncated ${entry.toolCalls.length} tool output(s)`,
				description: `${entry.summaryId} · ${summarizeToolNames(entry.toolCalls)} · saved ≈${formatChars(saved)}`,
			};
		});
}

export function janitorRunNoticeText(entry: SummaryIndexEntry): string {
	const saved = Math.max(0, entry.rawChars - entry.projectedChars);
	const lines = [
		`Context Janitor truncated ${entry.toolCalls.length} tool output(s).`,
		`${entry.summaryId} · ${summarizeToolNames(entry.toolCalls)} · saved ≈${formatChars(saved)} · ${entry.deciderModel}`,
	];
	const reasons = entry.toolCalls
		.map(record => record.janitorReason ? `${record.toolName}: ${record.janitorReason}` : undefined)
		.filter((reason): reason is string => typeof reason === "string")
		.slice(0, 3);
	if (reasons.length > 0) lines.push("", ...reasons.map(reason => `- ${truncateMiddle(reason, 180).replace(/\s+/g, " ")}`));
	if (entry.toolCalls.length > reasons.length && reasons.length > 0) lines.push(`- +${entry.toolCalls.length - reasons.length} more`);
	return lines.join("\n");
}

export function janitorRestoreNoticeText(count: number): string {
	return `Context Janitor restored ${count} run(s). Future model context will include those raw tool outputs again.`;
}

export class JanitorUndoPicker implements Component {
	#selectedIndex = 0;
	#checked = new Set<string>();

	constructor(
		private readonly items: UndoRunItem[],
		private readonly theme: ThemeLike,
		private readonly keybindings: KeybindingsLike,
		private readonly done: (result: string[] | undefined) => void,
	) {}

	invalidate(): void {
		// No cached layout.
	}

	render(width: number): string[] {
		const safeWidth = Math.max(1, width);
		const lines: string[] = [themeBold(this.theme, "Restore janitor actions"), ""];
		if (this.items.length === 0) {
			lines.push("  Nothing to restore.", "", themeFg(this.theme, "muted", "  Esc = close"));
			return lines.map(line => truncateToWidth(replaceTabs(line), safeWidth));
		}

		const maxVisible = Math.min(10, Math.max(4, this.items.length));
		const startIndex = Math.max(0, Math.min(this.#selectedIndex - Math.floor(maxVisible / 2), this.items.length - maxVisible));
		const endIndex = Math.min(startIndex + maxVisible, this.items.length);
		for (let i = startIndex; i < endIndex; i += 1) {
			const item = this.items[i];
			if (!item) continue;
			const selected = i === this.#selectedIndex;
			const checked = this.#checked.has(item.summaryId);
			const prefix = selected ? "›" : " ";
			const mark = checked ? "[x]" : "[ ]";
			const line = `${prefix} ${mark} ${item.label}`;
			lines.push(selected ? themeFg(this.theme, "accent", line) : line);
			if (selected) lines.push(themeFg(this.theme, "muted", `      ${item.description}`));
		}
		if (startIndex > 0 || endIndex < this.items.length) lines.push(themeFg(this.theme, "muted", `  (${this.#selectedIndex + 1}/${this.items.length})`));
		lines.push("", themeFg(this.theme, "muted", `  Space = toggle · a = all · Enter = restore ${this.#checked.size} selected · Esc = cancel`));
		return lines.map(line => truncateToWidth(replaceTabs(line), safeWidth));
	}

	handleInput(data: string): void {
		if (this.#matches(data, "tui.select.cancel") || this.#matches(data, "interrupt") || data === "\u001b" || data === "\u0003") {
			this.done(undefined);
			return;
		}
		if (this.items.length === 0) return;
		if (this.#matches(data, "tui.select.up") || data === "\u001b[A") {
			this.#selectedIndex = this.#selectedIndex === 0 ? this.items.length - 1 : this.#selectedIndex - 1;
			return;
		}
		if (this.#matches(data, "tui.select.down") || data === "\u001b[B") {
			this.#selectedIndex = this.#selectedIndex === this.items.length - 1 ? 0 : this.#selectedIndex + 1;
			return;
		}
		if (this.#matches(data, "tui.select.pageUp")) {
			this.#selectedIndex = Math.max(0, this.#selectedIndex - 10);
			return;
		}
		if (this.#matches(data, "tui.select.pageDown")) {
			this.#selectedIndex = Math.min(this.items.length - 1, this.#selectedIndex + 10);
			return;
		}
		if (data === " ") {
			this.#toggle(this.items[this.#selectedIndex]?.summaryId);
			return;
		}
		if (data.toLowerCase() === "a") {
			if (this.#checked.size === this.items.length) this.#checked.clear();
			else for (const item of this.items) this.#checked.add(item.summaryId);
			return;
		}
		if (this.#matches(data, "tui.select.confirm") || data === "\r" || data === "\n") {
			this.done([...this.#checked]);
		}
	}

	#toggle(summaryId: string | undefined): void {
		if (!summaryId) return;
		if (this.#checked.has(summaryId)) this.#checked.delete(summaryId);
		else this.#checked.add(summaryId);
	}

	#matches(data: string, action: string): boolean {
		try {
			return this.keybindings.matches(data, action);
		} catch {
			return false;
		}
	}
}

function compactJanitorNoticeLine(text: string, details: unknown): string {
	if (isRecord(details) && Array.isArray(details.summaryIds)) {
		const count = details.summaryIds.length;
		return `🧹 restored ${count} janitor run${count === 1 ? "" : "s"}`;
	}

	if (isRecord(details) && typeof details.toolCalls === "number") {
		const parts = [`🧹 truncated ${details.toolCalls} tool output${details.toolCalls === 1 ? "" : "s"}`];
		if (typeof details.rawChars === "number" && typeof details.projectedChars === "number") {
			parts.push(`saved ≈${formatChars(Math.max(0, details.rawChars - details.projectedChars))}`);
		}
		if (typeof details.summaryId === "string") parts.push(details.summaryId);
		return parts.join(" · ");
	}

	const firstLine = text.split("\n").map(line => line.trim()).find(Boolean);
	return firstLine ? `🧹 ${truncateMiddle(firstLine.replace(/\s+/g, " "), 120)}` : "🧹 Context Janitor";
}

export class JanitorNoticeComponent implements Component {
	constructor(
		private readonly text: string,
		private readonly details: unknown,
		private readonly theme: ThemeLike,
	) {}

	invalidate(): void {}

	render(width: number): string[] {
		const safeWidth = Math.max(1, width);
		if (isPiCompactEnabled()) {
			return [truncateToWidth(replaceTabs(themeFg(this.theme, "muted", compactJanitorNoticeLine(this.text, this.details))), safeWidth)];
		}

		const lines = this.text.split("\n");
		if (lines.length === 0) return [];
		return lines.map((line, index) => {
			const decorated = index === 0 ? themeFg(this.theme, "accent", line) : themeFg(this.theme, "muted", line);
			return truncateToWidth(replaceTabs(decorated), safeWidth);
		});
	}
}

export class HiddenMessageComponent implements Component {
	invalidate(): void {}
	render(_width: number): string[] {
		return [];
	}
}
