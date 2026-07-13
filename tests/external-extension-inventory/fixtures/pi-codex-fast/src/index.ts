import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { type ExtensionAPI, type ExtensionContext, getAgentDir } from "@earendil-works/pi-coding-agent";

interface CodexFastSettings {
	enabled: boolean;
	supportedModels: string[];
	showStatus: boolean;
}

const DEFAULT_SETTINGS: CodexFastSettings = {
	enabled: false,
	supportedModels: ["gpt-5.5"],
	showStatus: true,
};

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function parseSettings(raw: unknown): Partial<CodexFastSettings> {
	if (typeof raw === "boolean") return { enabled: raw };
	if (!isRecord(raw)) return {};
	const out: Partial<CodexFastSettings> = {};
	if (typeof raw.enabled === "boolean") out.enabled = raw.enabled;
	if (typeof raw.showStatus === "boolean") out.showStatus = raw.showStatus;
	if (Array.isArray(raw.supportedModels))
		out.supportedModels = raw.supportedModels.filter((s): s is string => typeof s === "string" && s.trim().length > 0);
	return out;
}

function pickSettings(parsed: Record<string, unknown>): unknown {
	const extensionSettings = parsed.extensionSettings;
	if (!isRecord(extensionSettings)) return undefined;
	return extensionSettings["codex-fast"];
}

function readSettingsFile(path: string): Partial<CodexFastSettings> {
	if (!existsSync(path)) return {};
	try {
		const parsed = JSON.parse(readFileSync(path, "utf-8")) as unknown;
		if (!isRecord(parsed)) return {};
		return parseSettings(pickSettings(parsed));
	} catch {
		return {};
	}
}

function loadSettings(cwd: string): CodexFastSettings {
	return {
		...DEFAULT_SETTINGS,
		...readSettingsFile(join(getAgentDir(), "settings.json")),
		...readSettingsFile(join(cwd, ".pi", "settings.json")),
	};
}

function isCodexFastActive(ctx: ExtensionContext, settings: CodexFastSettings): boolean {
	const model = ctx.model;
	return model?.provider === "openai-codex" && settings.enabled && settings.supportedModels.includes(model.id);
}

function updateStatus(ctx: ExtensionContext): void {
	const settings = loadSettings(ctx.cwd);
	const active = settings.showStatus && isCodexFastActive(ctx, settings);
	ctx.ui.setStatus("codex-fast", active ? ctx.ui.theme.fg("accent", "⚡") : undefined);
}

export default function codexFastExtension(pi: ExtensionAPI) {
	pi.on("session_start", async (_event, ctx) => {
		updateStatus(ctx);
	});

	pi.on("model_select", async (_event, ctx) => {
		updateStatus(ctx);
	});

	pi.registerCommand("codex-fast", {
		description: "Show Codex fast-mode status",
		handler: async (_args, ctx) => {
			const settings = loadSettings(ctx.cwd);
			const model = ctx.model ? `${ctx.model.provider}/${ctx.model.id}` : "none";
			const active = isCodexFastActive(ctx, settings) ? "on" : "off";
			const lines = [
				`codex-fast: ${active}`,
				`model: ${model}`,
				`enabled: ${settings.enabled}`,
				`supportedModels: ${settings.supportedModels.join(", ") || "(none)"}`,
				"config: ~/.pi/agent/settings.json#extensionSettings, .pi/settings.json#extensionSettings",
			];
			ctx.ui.notify(lines.join("\n"), "info");
		},
	});

	pi.on("before_provider_request", (event, ctx) => {
		const settings = loadSettings(ctx.cwd);
		if (!isCodexFastActive(ctx, settings)) return;
		if (!isRecord(event.payload) || event.payload.service_tier !== undefined) return;

		return {
			...event.payload,
			service_tier: "priority",
		};
	});
}
