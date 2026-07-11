// Regenerates tests/system-prompt-parity/oracle.json from Pi's real
// buildSystemPrompt / loadProjectContextFiles / tool definitions
// (ref/pi/packages/coding-agent). The private agent-session.ts
// normalization + _rebuildSystemPrompt composition is copied here (the
// established harness pattern for private wiring bodies). Run via
// scripts/system-prompt-oracle. Do not edit the oracle by hand.
//
// Determinism pins: TZ=UTC, PI_PACKAGE_DIR=/pi-rs-pkg (config.ts doc
// paths), a per-case fixed Date, and temp fixture roots substituted with
// {ROOT} in the recorded output.
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { buildSystemPrompt } from "../../ref/pi/packages/coding-agent/src/core/system-prompt.ts";
import { createAllToolDefinitions } from "../../ref/pi/packages/coding-agent/src/core/tools/index.ts";

// Both pins are read lazily (config.ts getPackageDir at call time, TZ at
// the first Date use), so setting them after the hoisted imports is safe.
process.env.TZ = "UTC";
process.env.PI_PACKAGE_DIR = "/pi-rs-pkg";

interface SkillCase {
	name: string;
	description: string;
	filePath: string;
	disableModelInvocation?: boolean;
}

interface SessionCase {
	name: string;
	toolNames: string[];
	customPrompt?: string;
	appendSystemPrompt?: string[];
	skills?: SkillCase[];
	tree: Record<string, string>;
	cwd: string;
	agentDir: string;
	nowMs: number;
}

interface RawCase {
	name: string;
	cwd: string;
	selectedTools?: string[];
	toolSnippets?: Record<string, string>;
	promptGuidelines?: string[];
	customPrompt?: string;
	appendSystemPrompt?: string;
	contextFiles?: Array<{ path: string; content: string }>;
	skills?: SkillCase[];
	nowMs: number;
}

const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as {
	session: SessionCase[];
	raw: RawCase[];
};

const RealDate = Date;
function withNow<T>(nowMs: number, fn: () => T): T {
	class FakeDate extends RealDate {
		constructor(...args: unknown[]) {
			if (args.length === 0) {
				super(nowMs);
			} else {
				// biome-ignore lint/suspicious/noExplicitAny: harness
				super(...(args as [any]));
			}
		}
	}
	(FakeDate as unknown as { now: () => number }).now = () => nowMs;
	(globalThis as { Date: unknown }).Date = FakeDate;
	try {
		return fn();
	} finally {
		(globalThis as { Date: unknown }).Date = RealDate;
	}
}

// Copied from resource-loader.ts loadContextFileFromDir /
// loadProjectContextFiles (importing the module transitively hits the
// vendored jiti's missing "./static" export under tsx; the function is
// self-contained), minus the chalk warning on unreadable files.
function loadContextFileFromDir(dir: string): { path: string; content: string } | null {
	const candidates = ["AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD"];
	for (const filename of candidates) {
		const filePath = join(dir, filename);
		if (existsSync(filePath)) {
			try {
				return {
					path: filePath,
					content: readFileSync(filePath, "utf-8"),
				};
			} catch {
				// keep scanning
			}
		}
	}
	return null;
}

function loadProjectContextFiles(options: {
	cwd: string;
	agentDir: string;
}): Array<{ path: string; content: string }> {
	const resolvedCwd = resolve(options.cwd);
	const resolvedAgentDir = resolve(options.agentDir);

	const contextFiles: Array<{ path: string; content: string }> = [];
	const seenPaths = new Set<string>();

	const globalContext = loadContextFileFromDir(resolvedAgentDir);
	if (globalContext) {
		contextFiles.push(globalContext);
		seenPaths.add(globalContext.path);
	}

	const ancestorContextFiles: Array<{ path: string; content: string }> = [];

	let currentDir = resolvedCwd;
	const root = resolve("/");

	while (true) {
		const contextFile = loadContextFileFromDir(currentDir);
		if (contextFile && !seenPaths.has(contextFile.path)) {
			ancestorContextFiles.unshift(contextFile);
			seenPaths.add(contextFile.path);
		}

		if (currentDir === root) break;

		const parentDir = resolve(currentDir, "..");
		if (parentDir === currentDir) break;
		currentDir = parentDir;
	}

	contextFiles.push(...ancestorContextFiles);

	return contextFiles;
}

// Copied from agent-session.ts _normalizePromptSnippet.
function normalizePromptSnippet(text: string | undefined): string | undefined {
	if (!text) return undefined;
	const oneLine = text
		.replace(/[\r\n]+/g, " ")
		.replace(/\s+/g, " ")
		.trim();
	return oneLine.length > 0 ? oneLine : undefined;
}

// Copied from agent-session.ts _normalizePromptGuidelines.
function normalizePromptGuidelines(guidelines: string[] | undefined): string[] {
	if (!guidelines || guidelines.length === 0) {
		return [];
	}
	const unique = new Set<string>();
	for (const guideline of guidelines) {
		const normalized = guideline.trim();
		if (normalized.length > 0) {
			unique.add(normalized);
		}
	}
	return Array.from(unique);
}

const oracle = {
	session: cases.session.map((c) => {
		const root = mkdtempSync(join(tmpdir(), "pi-rs-sysprompt-"));
		try {
			for (const [rel, content] of Object.entries(c.tree ?? {})) {
				const path = join(root, rel);
				mkdirSync(dirname(path), { recursive: true });
				writeFileSync(path, content);
			}
			const cwd = resolve(root, c.cwd);
			const agentDir = resolve(root, c.agentDir);
			const contextFiles = loadProjectContextFiles({ cwd, agentDir });
			// agent-session.ts _rebuildSystemPrompt over the base tool
			// definitions (the registered-definition registry).
			const defs = createAllToolDefinitions(cwd) as Record<
				string,
				{ promptSnippet?: string; promptGuidelines?: string[] }
			>;
			const validToolNames = (c.toolNames ?? []).filter((name) => name in defs);
			const toolSnippets: Record<string, string> = {};
			const promptGuidelines: string[] = [];
			for (const name of validToolNames) {
				const snippet = normalizePromptSnippet(defs[name].promptSnippet);
				if (snippet) toolSnippets[name] = snippet;
				promptGuidelines.push(...normalizePromptGuidelines(defs[name].promptGuidelines));
			}
			const appendList = c.appendSystemPrompt ?? [];
			const prompt = withNow(c.nowMs, () =>
				buildSystemPrompt({
					cwd,
					skills: (c.skills ?? []) as never,
					contextFiles,
					customPrompt: c.customPrompt || undefined,
					appendSystemPrompt: appendList.length > 0 ? appendList.join("\n\n") : undefined,
					selectedTools: validToolNames,
					toolSnippets,
					promptGuidelines,
				}),
			);
			return {
				name: c.name,
				contextFiles: contextFiles.map((file) => ({
					path: file.path.split(root).join("{ROOT}"),
					content: file.content,
				})),
				prompt: prompt.split(root).join("{ROOT}"),
			};
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	}),
	raw: cases.raw.map((c) => ({
		name: c.name,
		prompt: withNow(c.nowMs, () =>
			buildSystemPrompt({
				cwd: c.cwd,
				selectedTools: c.selectedTools ?? undefined,
				toolSnippets: c.toolSnippets ?? undefined,
				promptGuidelines: c.promptGuidelines ?? undefined,
				customPrompt: c.customPrompt ?? undefined,
				appendSystemPrompt: c.appendSystemPrompt ?? undefined,
				contextFiles: c.contextFiles ?? undefined,
				skills: (c.skills ?? undefined) as never,
			}),
		),
	})),
};

console.log(JSON.stringify(oracle, null, "\t"));
