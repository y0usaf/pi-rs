// Regenerates tests/tool-parity/oracle.json from Pi's real core/tools
// implementations (ref/pi/packages/coding-agent): each case builds a
// fixture tree in a temp root, runs the tool's prepareArguments +
// execute exactly the way the agent loop invokes it (toolCallId, args,
// signal, onUpdate, ctx), and records the result/error plus filesystem
// effects. Run via scripts/tool-oracle. Do not edit the oracle by hand.
//
// Determinism pins: PI_CODING_AGENT_DIR points at an empty temp dir so
// ensureTool resolves the system rg/fd from PATH (the nix shell provides
// them); temp roots are substituted with {ROOT} and the bash tool's
// persisted full-output path with {FULL_OUTPUT} in the recorded output.
// Grep/find cases are restricted to deterministic outputs (single
// matching file) because rg/fd traverse directories in parallel; the
// multi-file ordering behavior stays covered by pi-rs's behavioral tests.
// The read image cases cover both auto-resize outcomes: a small PNG
// within all limits (image-resize-core: wasResized=false, original
// bytes untouched) and an oversized PNG that pi resizes through Photon
// (pi-rs through the pi.image photon-slice port — byte parity pinned by
// tests/image-parity).
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, sep } from "node:path";

// The env pin must land before the tools modules load (tools-manager.ts
// computes TOOLS_DIR at import time), so the ref/pi imports are dynamic
// inside main() — the tsx cjs transform rejects top-level await.
process.env.PI_CODING_AGENT_DIR = mkdtempSync(join(tmpdir(), "pi-rs-tool-oracle-agentdir-"));

type Generated = { gen: "lines"; count: number; prefix?: string; suffix?: string } | {
	gen: "repeat";
	unit: string;
	count: number;
};

interface Case {
	name: string;
	tool: "read" | "bash" | "edit" | "write" | "grep" | "find" | "ls";
	tree: Record<string, string | Generated>;
	binary?: Record<string, string>;
	args: Record<string, unknown>;
	abort?: "pre";
	abortAfterMs?: number;
	model?: { id: string; input: string[] };
	recordFs?: boolean;
	recordFullOutput?: boolean;
}

function generate(spec: Generated): string {
	if (spec.gen === "repeat") return spec.unit.repeat(spec.count);
	const lines: string[] = [];
	for (let i = 1; i <= spec.count; i++) {
		lines.push(`${spec.prefix ?? ""}${i}${spec.suffix ?? ""}`);
	}
	return lines.join("\n");
}

function materialize(root: string, c: Case): void {
	for (const [rel, value] of Object.entries(c.tree ?? {})) {
		const path = join(root, rel);
		if (rel.endsWith("/")) {
			mkdirSync(path, { recursive: true });
			continue;
		}
		mkdirSync(dirname(path), { recursive: true });
		writeFileSync(path, typeof value === "string" ? value : generate(value));
	}
	for (const [rel, base64] of Object.entries(c.binary ?? {})) {
		const path = join(root, rel);
		mkdirSync(dirname(path), { recursive: true });
		writeFileSync(path, Buffer.from(base64, "base64"));
	}
}

function walkFiles(root: string): Record<string, string> {
	const out: Record<string, string> = {};
	const visit = (dir: string): void => {
		for (const entry of readdirSync(dir).sort()) {
			const path = join(dir, entry);
			if (statSync(path).isDirectory()) visit(path);
			else out[relative(root, path).split(sep).join("/")] = readFileSync(path, "utf-8");
		}
	};
	visit(root);
	return out;
}

async function main(): Promise<void> {
	const { createReadToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/read.ts"
	);
	const { createBashToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/bash.ts"
	);
	const { createEditToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/edit.ts"
	);
	const { createWriteToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/write.ts"
	);
	const { createGrepToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/grep.ts"
	);
	const { createFindToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/find.ts"
	);
	const { createLsToolDefinition } = await import(
		"../../ref/pi/packages/coding-agent/src/core/tools/ls.ts"
	);

	function createDefinition(tool: Case["tool"], cwd: string) {
		switch (tool) {
			case "read":
				return createReadToolDefinition(cwd);
			case "bash":
				return createBashToolDefinition(cwd);
			case "edit":
				return createEditToolDefinition(cwd);
			case "write":
				return createWriteToolDefinition(cwd);
			case "grep":
				return createGrepToolDefinition(cwd);
			case "find":
				return createFindToolDefinition(cwd);
			case "ls":
				return createLsToolDefinition(cwd);
		}
	}

	const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as { cases: Case[] };

	const results: unknown[] = [];
	for (const c of cases.cases) {
	const root = mkdtempSync(join(tmpdir(), "pi-rs-tool-parity-"));
	try {
		materialize(root, c);
		const definition = createDefinition(c.tool, root) as {
			prepareArguments?: (args: unknown) => unknown;
			execute: (
				id: string,
				args: unknown,
				signal?: AbortSignal,
				onUpdate?: unknown,
				ctx?: unknown,
			) => Promise<{ content: unknown; details?: unknown }>;
		};
		const controller = new AbortController();
		if (c.abort === "pre") controller.abort();
		let abortTimer: NodeJS.Timeout | undefined;
		if (typeof c.abortAfterMs === "number") {
			abortTimer = setTimeout(() => controller.abort(), c.abortAfterMs);
		}
		let ok = true;
		let payload: unknown;
		try {
			let args: unknown = c.args;
			if (definition.prepareArguments) args = definition.prepareArguments(args);
			payload = await definition.execute(
				"parity-call",
				args,
				controller.signal,
				undefined,
				c.model ? { model: c.model } : undefined,
			);
		} catch (error) {
			ok = false;
			payload = error instanceof Error ? error.message : String(error);
		} finally {
			if (abortTimer) clearTimeout(abortTimer);
		}

		let fullOutput: string | undefined;
		let fullOutputPath: string | undefined;
		if (
			ok &&
			payload &&
			typeof payload === "object" &&
			(payload as { details?: { fullOutputPath?: string } }).details?.fullOutputPath
		) {
			fullOutputPath = (payload as { details: { fullOutputPath: string } }).details.fullOutputPath;
			if (c.recordFullOutput) fullOutput = readFileSync(fullOutputPath, "utf-8");
		}

		const substitute = (text: string): string => {
			let out = text.split(root).join("{ROOT}");
			if (fullOutputPath) out = out.split(fullOutputPath).join("{FULL_OUTPUT}");
			return out;
		};

		const entry: Record<string, unknown> = { name: c.name, ok };
		if (ok) {
			entry.result = JSON.parse(substitute(JSON.stringify(payload)));
		} else {
			entry.error = substitute(payload as string);
		}
		if (c.recordFs) entry.files = walkFiles(root);
		if (fullOutput !== undefined) entry.fullOutput = fullOutput;
		if (fullOutputPath) rmSync(fullOutputPath, { force: true });
		results.push(entry);
	} finally {
		rmSync(root, { recursive: true, force: true });
	}
	}

	console.log(JSON.stringify({ cases: results }, null, "\t"));
	process.exit(0);
}

void main();
