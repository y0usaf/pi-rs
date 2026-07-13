import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { getSettingsListTheme, getAgentDir, type ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Container, type SettingItem, SettingsList, Text } from "@earendil-works/pi-tui";

// ── Types & constants ──────────────────────────────────────────────

interface ToolSettingsFile {
	version: number;
	disabledTools: string[];
}

interface ToolRecord {
	name: string;
	sourceInfo?: {
		source?: string;
		scope?: string;
	};
}

const SETTINGS_VERSION = 1;
const SETTINGS_PATH = join(getAgentDir(), "tool-settings.json");
const ALLOWED = "allowed";
const BLOCKED = "blocked";
const BLOCKED_EXTERNALLY = "blocked (external)";

// ── Helpers ────────────────────────────────────────────────────────

function uniqueSorted(arr: string[]): string[] {
	return [...new Set(arr)].sort((a, b) => a.localeCompare(b));
}

function toStringArray(value: unknown): string[] {
	if (!Array.isArray(value)) return [];
	return value.filter((v): v is string => typeof v === "string").map((s) => s.trim()).filter(Boolean);
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function normalizeTool(tool: unknown): ToolRecord | undefined {
	if (typeof tool === "string") {
		const name = tool.trim();
		return name ? { name } : undefined;
	}
	if (!isRecord(tool) || typeof tool.name !== "string") return undefined;

	const name = tool.name.trim();
	if (!name) return undefined;

	const sourceInfo = isRecord(tool.sourceInfo) ? {
		source: typeof tool.sourceInfo.source === "string" ? tool.sourceInfo.source : undefined,
		scope: typeof tool.sourceInfo.scope === "string" ? tool.sourceInfo.scope : undefined,
	} : undefined;

	return sourceInfo ? { name, sourceInfo } : { name };
}

function getAllToolRecords(pi: ExtensionAPI): ToolRecord[] {
	const rawTools = pi.getAllTools() as unknown;
	if (!Array.isArray(rawTools)) return [];

	const seen = new Set<string>();
	const tools: ToolRecord[] = [];
	for (const rawTool of rawTools) {
		const tool = normalizeTool(rawTool);
		if (!tool || seen.has(tool.name)) continue;
		seen.add(tool.name);
		tools.push(tool);
	}
	return tools;
}

// ── Settings I/O ───────────────────────────────────────────────────

let disabledTools = new Set<string>();
let lastWarning: string | undefined;
let lastSaveError: string | undefined;

function parseSettings(raw: string): { disabledTools: string[]; warning?: string } {
	const parsed: unknown = JSON.parse(raw);

	if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
		return { disabledTools: [], warning: `Ignoring invalid settings in ${SETTINGS_PATH}: expected object` };
	}

	const obj = parsed as Record<string, unknown>;
	if (obj.version !== SETTINGS_VERSION) {
		return { disabledTools: [], warning: `Ignoring unsupported settings version in ${SETTINGS_PATH}: ${String(obj.version)}` };
	}

	return { disabledTools: uniqueSorted(toStringArray(obj.disabledTools)) };
}

async function loadSettings(): Promise<void> {
	try {
		const raw = await readFile(SETTINGS_PATH, "utf-8");
		try {
			const result = parseSettings(raw);
			disabledTools = new Set(result.disabledTools);
			lastWarning = result.warning;
			if (lastWarning) console.warn(`[pi-tool-management] ${lastWarning}`);
		} catch (e) {
			const msg = `Failed to parse ${SETTINGS_PATH}: ${e instanceof Error ? e.message : String(e)}`;
			lastWarning = msg;
			console.warn(`[pi-tool-management] ${msg}`);
		}
	} catch (e) {
		const err = e as NodeJS.ErrnoException;
		if (err?.code !== "ENOENT") {
			const msg = `Failed to load ${SETTINGS_PATH}: ${err.message}`;
			lastWarning = msg;
			console.warn(`[pi-tool-management] ${msg}`);
		}
		// ENOENT: no file yet, keep current disabledTools (empty on first load)
	}
}

async function saveSettings(): Promise<void> {
	const file: ToolSettingsFile = { version: SETTINGS_VERSION, disabledTools: uniqueSorted([...disabledTools]) };
	try {
		await mkdir(getAgentDir(), { recursive: true });
		await writeFile(SETTINGS_PATH, `${JSON.stringify(file, null, 2)}\n`, "utf-8");
		lastSaveError = undefined;
	} catch (e) {
		const msg = `Failed to save ${SETTINGS_PATH}: ${e instanceof Error ? e.message : String(e)}`;
		lastSaveError = msg;
		console.error(`[pi-tool-management] ${msg}`);
	}
}

// ── Tool sorting & enforcement ─────────────────────────────────────

function getToolCategory(tool: ToolRecord): string {
	if (tool.sourceInfo?.source === "builtin") return "Built-in";
	if (tool.sourceInfo?.source === "sdk") return "SDK";
	if (tool.sourceInfo?.scope === "project") return "Project extension";
	if (tool.sourceInfo?.scope === "user") return "User extension";
	return tool.sourceInfo ? "Extension" : "Tool";
}

function sortTools(tools: ToolRecord[]): ToolRecord[] {
	const rank = (t: ToolRecord) =>
		t.sourceInfo?.source === "builtin" ? 0 :
		t.sourceInfo?.source === "sdk" ? 1 :
		t.sourceInfo?.scope === "project" ? 2 :
		t.sourceInfo?.scope === "user" ? 3 : 4;
	return [...tools].sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
}

function getToolValue(name: string, activeTools: Set<string>): string {
	if (disabledTools.has(name)) return BLOCKED;
	if (!activeTools.has(name)) return BLOCKED_EXTERNALLY;
	return ALLOWED;
}

function getToolValues(currentValue: string): string[] {
	if (currentValue === BLOCKED) return [BLOCKED, ALLOWED];
	if (currentValue === BLOCKED_EXTERNALLY) return [BLOCKED_EXTERNALLY, BLOCKED];
	return [ALLOWED, BLOCKED];
}

async function enforceDisabledTools(pi: ExtensionAPI): Promise<void> {
	const allNames = new Set(getAllToolRecords(pi).map((t) => t.name));
	if (allNames.size === 0) return;

	const active = pi.getActiveTools().filter((n) => allNames.has(n));
	const filtered = active.filter((n) => !disabledTools.has(n));
	if (active.length !== filtered.length || active.some((n, i) => n !== filtered[i])) {
		await pi.setActiveTools(filtered);
	}
}

async function reloadAndEnforce(pi: ExtensionAPI): Promise<void> {
	await loadSettings();
	await enforceDisabledTools(pi);
}

// ── Extension entry point ──────────────────────────────────────────

export default function toolManagementExtension(pi: ExtensionAPI) {
	// /tools command — interactive SettingsList UI
	pi.registerCommand("tools", {
		description: "Manage this extension's global disabled-tools list (~/.pi/agent/tool-settings.json)",
		handler: async (_args, ctx) => {
			await reloadAndEnforce(pi);

			const allTools = sortTools(getAllToolRecords(pi));
			if (allTools.length === 0) {
				ctx.ui.notify("No tools available", "info");
				return;
			}

			await ctx.ui.custom((tui, theme, _kb, done) => {
				const activeTools = new Set(pi.getActiveTools());
				const blockedExternallyNames = allTools
					.map((tool) => tool.name)
					.filter((name) => !disabledTools.has(name) && !activeTools.has(name));
				const items: SettingItem[] = allTools.map((tool) => {
					const currentValue = getToolValue(tool.name, activeTools);
					const isBlockedExternally = currentValue === BLOCKED_EXTERNALLY;
					return {
						id: tool.name,
						label: `${tool.name} · ${getToolCategory(tool)}`,
						description: isBlockedExternally
							? "Blocked (external)."
							: undefined,
						currentValue,
						values: getToolValues(currentValue),
					};
				});

				const container = new Container();
				container.addChild(new Text(theme.fg("accent", theme.bold("Tool Management"))));
				container.addChild(new Text(theme.fg("dim", SETTINGS_PATH)));
				container.addChild(new Text(theme.fg("muted", "This menu edits this extension's global disabled-tools list.")));
				container.addChild(new Text(theme.fg("muted", "Blocked = disabled by this extension. Blocked (external) = hidden by another extension or runtime mode.")));
				if (blockedExternallyNames.length > 0) {
					container.addChild(new Text(theme.fg("warning", `Blocked (external) now: ${blockedExternallyNames.join(", ")}`)));
				}
				container.addChild(new Text(theme.fg("muted", "Scans built-in + extension tools each time this menu opens.")));
				container.addChild(new Text(theme.fg("muted", "Close + reopen to refresh tools added while this menu is open.")));

				const settingsList = new SettingsList(
					items,
					Math.min(items.length + 2, 15),
					getSettingsListTheme(),
					(id, newValue) => {
						if (newValue === BLOCKED) {
							disabledTools.add(id);
						} else {
							disabledTools.delete(id);
						}

						void enforceDisabledTools(pi)
							.then(() => {
								settingsList.updateValue(id, getToolValue(id, new Set(pi.getActiveTools())));
								tui.requestRender();
							})
							.catch((e) => {
								ctx.ui.notify(`Failed to apply tool changes: ${e instanceof Error ? e.message : String(e)}`, "error");
							});
						void saveSettings().then(() => {
							if (lastSaveError) ctx.ui.notify(`${lastSaveError}\nChanges remain applied in this session.`, "error");
						});
					},
					() => done(undefined),
				);

				container.addChild(settingsList);
				container.addChild(new Text(theme.fg("dim", "↑↓ navigate • ←/→ toggle • esc close")));

				return {
					render: (width: number) => container.render(width),
					invalidate: () => container.invalidate(),
					handleInput: (data: string) => { settingsList.handleInput?.(data); tui.requestRender(); },
				};
			});
		},
	});

	// /tools-status command — diagnostic info
	pi.registerCommand("tools-status", {
		description: "Show tool-settings.json status",
		handler: async (_args, ctx) => {
			await reloadAndEnforce(pi);

			const allTools = sortTools(getAllToolRecords(pi));
			const activeTools = new Set(pi.getActiveTools());
			const knownNames = new Set(allTools.map((t) => t.name));
			const activeKnown = [...activeTools].filter((n) => knownNames.has(n));
			const disabled = uniqueSorted([...disabledTools]);
			const unresolved = disabled.filter((n) => !knownNames.has(n));
			const blockedExternallyNames = allTools
				.map((tool) => tool.name)
				.filter((name) => !disabledTools.has(name) && !activeTools.has(name));

			const lines = [
				`settings: ${SETTINGS_PATH}`,
				`currentlyActiveAfterAllFilters: ${activeKnown.length}/${allTools.length}`,
				`disabledTools: ${disabled.join(", ") || "(none)"}`,
				`blockedExternally: ${blockedExternallyNames.join(", ") || "(none)"}`,
				"note: blockedExternally means a known tool this extension allows is shown as blocked (external) when it is absent from the current runtime active-tool set (another extension or runtime mode may be hiding it)",
			];
			if (unresolved.length > 0) lines.push(`unresolvedDisabledTools: ${unresolved.join(", ")}`);
			if (lastWarning) lines.push(`loadWarning: ${lastWarning}`);
			if (lastSaveError) lines.push(`saveError: ${lastSaveError}`);

			ctx.ui.notify(lines.join("\n"), lastSaveError ? "error" : lastWarning ? "warning" : "info");
		},
	});

	// Enforce disabled tools on all 4 lifecycle hooks
	for (const event of ["session_start", "session_tree", "before_agent_start", "before_provider_request"] as const) {
		pi.on(event, () => reloadAndEnforce(pi));
	}
}
