import { CustomEditor, type ExtensionAPI, type ExtensionContext } from "@earendil-works/pi-coding-agent";
import { truncateToWidth, visibleWidth } from "@earendil-works/pi-tui";

type Theme = ExtensionContext["ui"]["theme"];
type Level = "off" | "minimal" | "low" | "medium" | "high" | "xhigh";
type Footer = {
	getGitBranch(): string | null;
	getExtensionStatuses(): ReadonlyMap<string, string>;
	getAvailableProviderCount(): number;
	onBranchChange(callback: () => void): () => void;
};

const ANSI = /\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\)|_[^\x07]*(?:\x07|\x1b\\))/g;
const COLOR: Record<Level, Parameters<Theme["fg"]>[0]> = {
	off: "thinkingOff", minimal: "thinkingMinimal", low: "thinkingLow", medium: "thinkingMedium", high: "thinkingHigh", xhigh: "thinkingXhigh",
};
const compact = (n: number) => n < 1e3 ? `${n}` : n < 1e4 ? `${(n / 1e3).toFixed(1)}k` : n < 1e6 ? `${Math.round(n / 1e3)}k` : n < 1e7 ? `${(n / 1e6).toFixed(1)}M` : `${Math.round(n / 1e6)}M`;
const singleLine = (s: string) => s.replace(/[\r\n\t]/g, " ").replace(/ +/g, " ").trim();

function statusColor(theme: Theme, text: string): string {
	ANSI.lastIndex = 0;
	return ANSI.test(text) ? text : theme.fg("dim", text);
}

function footerStats(ctx: ExtensionContext, theme: Theme): string {
	let input = 0, output = 0, read = 0, write = 0, cost = 0;
	for (const entry of ctx.sessionManager.getEntries()) {
		if (entry.type !== "message" || entry.message.role !== "assistant") continue;
		input += entry.message.usage?.input ?? 0;
		output += entry.message.usage?.output ?? 0;
		read += entry.message.usage?.cacheRead ?? 0;
		write += entry.message.usage?.cacheWrite ?? 0;
		cost += entry.message.usage?.cost?.total ?? 0;
	}
	const context = ctx.getContextUsage();
	const contextWindow = context?.contextWindow ?? ctx.model?.contextWindow ?? 0;
	const pct = context?.percent ?? 0;
	const pctText = context?.percent !== null ? `${pct.toFixed(1)}%/${compact(contextWindow)} (auto)` : `?/${compact(contextWindow)} (auto)`;
	const sub = ctx.model ? ctx.modelRegistry.isUsingOAuth(ctx.model) : false;
	return [
		input && `↑${compact(input)}`,
		output && `↓${compact(output)}`,
		read && `R${compact(read)}`,
		write && `W${compact(write)}`,
		(cost || sub) && `$${cost.toFixed(3)}${sub ? " (sub)" : ""}`,
		pct > 90 ? theme.fg("error", pctText) : pct > 70 ? theme.fg("warning", pctText) : pctText,
	].filter(Boolean).join(" ");
}


function borders(pi: ExtensionAPI, ctx: ExtensionContext, footer: Footer, theme: Theme, width: number) {
	const value = ctx.model?.reasoning ? pi.getThinkingLevel() : undefined;
	const level: Level = value && value in COLOR ? value as Level : "off";
	const color = COLOR[level], fill = theme.fg(color, "─");
	const box = (...parts: Array<string | undefined | false>) => parts.filter(Boolean).join(theme.fg("dim", " • ")) || undefined;
	const line = (boxes: string[]) => {
		const fixed = boxes.reduce((sum, part) => sum + visibleWidth(part), 0);
		const gap = Math.max(0, boxes.length < 2 ? width - fixed : Math.floor((width - fixed) / (boxes.length - 1)));
		const text = boxes.length ? boxes.join(fill.repeat(Math.max(1, gap))) + (boxes.length === 1 ? fill.repeat(gap) : "") : fill.repeat(width);
		const clipped = truncateToWidth(text, width, "…");
		return clipped + " ".repeat(Math.max(0, width - visibleWidth(clipped)));
	};

	const home = process.env.HOME || process.env.USERPROFILE;
	let cwd = ctx.sessionManager.getCwd();
	if (home && cwd.startsWith(home)) cwd = `~${cwd.slice(home.length)}`;
	const branch = footer.getGitBranch();
	if (branch) cwd += ` (${branch})`;
	const session = ctx.sessionManager.getSessionName();
	if (session) cwd += ` • ${session}`;
	if (cwd.length > width) {
		const half = Math.floor(width / 2) - 1;
		cwd = half > 1 ? `${cwd.slice(0, half)}…${cwd.slice(-(half - 1))}` : cwd.slice(0, Math.max(1, width));
	}

	const model = ctx.model?.id || "no-model";
	const modelText = footer.getAvailableProviderCount() > 1 && ctx.model ? `(${ctx.model.provider}) ${model}` : model;
	const thinking = ctx.model?.reasoning ? level === "off" ? "thinking off" : level : undefined;
	return {
		top: line([box(theme.fg("dim", cwd)), box(theme.fg("dim", footerStats(ctx, theme)))].filter(Boolean) as string[]),
		bottom: line([
			box(theme.fg("dim", modelText), thinking && theme.fg(level === "off" ? "dim" : color, thinking)),
			...[...footer.getExtensionStatuses().entries()].sort(([a], [b]) => a.localeCompare(b)).map(([, value]) => {
				const status = singleLine(value);
				return status ? box(statusColor(theme, status)) : undefined;
			}),
		].filter(Boolean) as string[]),
	};
}


class MinimalEditor extends CustomEditor {
	constructor(private readonly getBorders: (width: number) => { top: string; bottom: string }, ...args: ConstructorParameters<typeof CustomEditor>) { super(...args); }
	render(width: number): string[] {
		const lines = super.render(width);
		if (width < 4 || !lines.length) return lines;
		const bottom = lines.findIndex((line, index) => index > 0 && /^─+$/.test(line.replace(ANSI, "").trim()));
		const chrome = this.getBorders(width);
		return [chrome.top, ...(bottom < 0 ? lines.slice(1) : [...lines.slice(1, bottom), ...lines.slice(bottom + 1)]), chrome.bottom];
	}
}

export default function minimalEditor(pi: ExtensionAPI) {
	pi.on("session_start", (_event, ctx) => ctx.ui.setFooter((tui, theme, footer: Footer) => {
		const unsubscribe = footer.onBranchChange(() => tui.requestRender());
		ctx.ui.setEditorComponent((editorTui, editorTheme, keybindings) => new MinimalEditor((width) => borders(pi, ctx, footer, theme, width), editorTui, editorTheme, keybindings, { paddingX: 0 }));
		return { dispose: () => { unsubscribe(); ctx.ui.setEditorComponent(undefined); }, invalidate() {}, render: () => [] };
	}));
	pi.on("session_shutdown", (_event, ctx) => { ctx.ui.setEditorComponent(undefined); ctx.ui.setFooter(undefined); });
}
