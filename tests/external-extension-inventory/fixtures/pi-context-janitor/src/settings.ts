import { mkdir, readFile, writeFile } from "node:fs/promises";
import { DEFAULT_SETTINGS, SETTINGS_DIR, SETTINGS_PATH } from "./constants.js";
import type { JanitorSettings } from "./types.js";
import { isRecord } from "./utils.js";

function parseSettings(raw: unknown): Partial<JanitorSettings> {
	if (typeof raw === "boolean") return { enabled: raw };
	if (!isRecord(raw)) return {};
	return typeof raw.enabled === "boolean" ? { enabled: raw.enabled } : {};
}

export async function loadSettings(): Promise<{ settings: JanitorSettings; error?: string }> {
	try {
		const raw = await readFile(SETTINGS_PATH, "utf-8");
		try {
			return { settings: { ...DEFAULT_SETTINGS, ...parseSettings(JSON.parse(raw) as unknown) } };
		} catch (error) {
			return {
				settings: { ...DEFAULT_SETTINGS },
				error: `Failed to parse ${SETTINGS_PATH}: ${error instanceof Error ? error.message : String(error)}`,
			};
		}
	} catch (error) {
		const err = error as NodeJS.ErrnoException;
		if (err?.code === "ENOENT") return { settings: { ...DEFAULT_SETTINGS } };
		return {
			settings: { ...DEFAULT_SETTINGS },
			error: `Failed to read ${SETTINGS_PATH}: ${err?.message ?? String(error)}`,
		};
	}
}

export async function saveSettings(settings: JanitorSettings): Promise<void> {
	await mkdir(SETTINGS_DIR, { recursive: true });
	await writeFile(SETTINGS_PATH, `${JSON.stringify(settings, null, 2)}\n`, "utf-8");
}
