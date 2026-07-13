import { PI_COMPACT_GLOBAL_KEY } from "./constants.js";
import type { ThemeLike } from "./types.js";

export function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function formatCount(value: number): string {
	if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
	if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
	return String(value);
}

export function formatChars(value: number): string {
	return `${formatCount(value)}ch`;
}

export function isPiCompactEnabled(): boolean {
	return (globalThis as Record<string, unknown>)[PI_COMPACT_GLOBAL_KEY] === true;
}

export function truncateMiddle(text: string, maxChars: number): string {
	if (text.length <= maxChars) return text;
	if (maxChars <= 1) return "…".slice(0, Math.max(0, maxChars));
	const marker = `\n...[truncated ${text.length - maxChars} chars]...\n`;
	if (marker.length >= maxChars) return `${text.slice(0, maxChars - 1)}…`;
	const head = Math.max(0, Math.floor((maxChars - marker.length) * 0.58));
	const tail = Math.max(0, maxChars - marker.length - head);
	return `${text.slice(0, head)}${marker}${text.slice(text.length - tail)}`;
}

export function safeJson(value: unknown, maxChars = 4_000): string {
	try {
		const text = JSON.stringify(value, null, 2);
		return truncateMiddle(text === undefined ? "undefined" : text, maxChars);
	} catch {
		return truncateMiddle(String(value), maxChars);
	}
}

export function stableJson(value: unknown): string {
	if (value === undefined) return "undefined";
	if (value === null || typeof value !== "object") return JSON.stringify(value) ?? String(value);
	if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
	const record = value as Record<string, unknown>;
	return `{${Object.keys(record).sort().map(key => `${JSON.stringify(key)}:${stableJson(record[key])}`).join(",")}}`;
}

export function textFromContent(content: unknown): string {
	if (typeof content === "string") return content;
	if (!Array.isArray(content)) return "";
	const parts: string[] = [];
	for (const part of content) {
		if (!isRecord(part)) continue;
		if (part.type === "text" && typeof part.text === "string") parts.push(part.text);
		else if (part.type === "image") parts.push("[image]");
		else if (part.type === "thinking" && typeof part.thinking === "string") parts.push(part.thinking);
		else if (part.type === "toolCall") parts.push(`[toolCall ${String(part.name ?? "")}]`);
	}
	return parts.join("\n");
}

export function splitArgs(args: string): string[] {
	return args.trim().split(/\s+/).filter(Boolean);
}

export function replaceTabs(text: string): string {
	return text.replace(/\t/g, "    ");
}

export function themeFg(theme: ThemeLike, color: string, text: string): string {
	try {
		return theme.fg(color as never, text);
	} catch {
		return text;
	}
}

export function themeBold(theme: ThemeLike, text: string): string {
	try {
		return theme.bold(text);
	} catch {
		return text;
	}
}

export function shortTimestamp(iso: string): string {
	const date = new Date(iso);
	if (Number.isNaN(date.getTime())) return iso;
	return date.toLocaleString(undefined, { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
}
