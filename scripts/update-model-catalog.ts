#!/usr/bin/env bun
/**
 * Normalize Pi's generated model catalog into pi-rs's reviewed data snapshot.
 * Network/source discovery lives here, never in the runtime registry.
 */

import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const DEFAULT_REPOSITORY = "https://github.com/earendil-works/pi.git";
const DEFAULT_REF = "main";
const DEFAULT_SOURCE_PATH = "packages/ai/src/models.generated.ts";
const DEFAULT_OUTPUT = "crates/pi-rs-ai/data/models.json";
const DEFAULT_PROVENANCE = "crates/pi-rs-ai/data/models.provenance.json";
const DEFAULT_OVERRIDES = "scripts/model-catalog-overrides.json";
const ACCEPTED_APIS = new Set([
	"openai-completions",
	"mistral-conversations",
	"openai-responses",
	"azure-openai-responses",
	"openai-codex-responses",
	"anthropic-messages",
	"bedrock-converse-stream",
	"google-generative-ai",
	"google-vertex",
]);
const MODEL_KEYS = new Set([
	"id",
	"name",
	"api",
	"provider",
	"baseUrl",
	"reasoning",
	"thinkingLevelMap",
	"input",
	"cost",
	"contextWindow",
	"maxTokens",
	"headers",
	"compat",
]);
const COST_RATE_KEYS = new Set(["input", "output", "cacheRead", "cacheWrite"]);
const COST_KEYS = new Set([...COST_RATE_KEYS, "tiers"]);
const COST_TIER_KEYS = new Set([...COST_RATE_KEYS, "inputTokensAbove"]);
const THINKING_KEYS = new Set(["off", "minimal", "low", "medium", "high", "xhigh", "max"]);

type JsonObject = Record<string, unknown>;
type CatalogEntry = { provider: string; models: JsonObject[] };
type Options = {
	repository: string;
	ref: string;
	revision?: string;
	source?: string;
	sourcePath: string;
	output: string;
	provenance: string;
	overrides: string;
	summaryOutput?: string;
};

type Override = {
	provider: string;
	model: string;
	reason: string;
	set?: JsonObject;
	remove?: string[];
};

function usage(): never {
	console.error(`usage: update-model-catalog.ts [options]

  --source PATH          local Pi checkout or generated .ts file (offline)
  --repository URL       upstream git repository (${DEFAULT_REPOSITORY})
  --ref REF              upstream ref when --revision is absent (${DEFAULT_REF})
  --revision REV         exact upstream revision
  --source-path PATH     catalog path within checkout (${DEFAULT_SOURCE_PATH})
  --output PATH          normalized catalog output (${DEFAULT_OUTPUT})
  --provenance PATH      provenance output (${DEFAULT_PROVENANCE})
  --overrides PATH       reviewed metadata overrides (${DEFAULT_OVERRIDES})
  --summary-output PATH  write PR-ready inventory summary`);
	process.exit(2);
}

function parseArgs(args: string[]): Options {
	const options: Options = {
		repository: DEFAULT_REPOSITORY,
		ref: DEFAULT_REF,
		sourcePath: DEFAULT_SOURCE_PATH,
		output: DEFAULT_OUTPUT,
		provenance: DEFAULT_PROVENANCE,
		overrides: DEFAULT_OVERRIDES,
	};
	const keys: Record<string, keyof Options> = {
		"--source": "source",
		"--repository": "repository",
		"--ref": "ref",
		"--revision": "revision",
		"--source-path": "sourcePath",
		"--output": "output",
		"--provenance": "provenance",
		"--overrides": "overrides",
		"--summary-output": "summaryOutput",
	};
	for (let index = 0; index < args.length; index++) {
		if (args[index] === "--help" || args[index] === "-h") usage();
		const key = keys[args[index]];
		const value = args[++index];
		if (!key || !value) usage();
		(options as unknown as Record<string, string>)[key] = value;
	}
	return options;
}

function fail(message: string): never {
	throw new Error(`model catalog: ${message}`);
}

function object(value: unknown, where: string): JsonObject {
	if (value === null || Array.isArray(value) || typeof value !== "object") {
		fail(`${where} must be an object`);
	}
	return value as JsonObject;
}

function string(value: unknown, where: string): string {
	if (typeof value !== "string" || value.length === 0) fail(`${where} must be a non-empty string`);
	return value;
}

function finiteNumber(value: unknown, where: string): number {
	if (typeof value !== "number" || !Number.isFinite(value)) fail(`${where} must be a finite number`);
	return value;
}

function rejectUnknownKeys(value: JsonObject, accepted: Set<string>, where: string): void {
	const unknown = Object.keys(value).filter((key) => !accepted.has(key));
	if (unknown.length > 0) fail(`${where} has unknown field(s): ${unknown.join(", ")}`);
}

function validateModel(modelValue: unknown, providerKey: string, modelKey: string): JsonObject {
	const where = `${providerKey}/${modelKey}`;
	const model = object(modelValue, where);
	rejectUnknownKeys(model, MODEL_KEYS, where);
	if (string(model.id, `${where}.id`) !== modelKey) fail(`${where}.id does not match its catalog key`);
	string(model.name, `${where}.name`);
	const api = string(model.api, `${where}.api`);
	if (!ACCEPTED_APIS.has(api)) fail(`${where}.api uses unsupported wire protocol ${JSON.stringify(api)}`);
	if (string(model.provider, `${where}.provider`) !== providerKey) {
		fail(`${where}.provider does not match its catalog key`);
	}
	if (typeof model.baseUrl !== "string") fail(`${where}.baseUrl must be a string`);
	if (typeof model.reasoning !== "boolean") fail(`${where}.reasoning must be boolean`);
	if (!Array.isArray(model.input) || model.input.length === 0) fail(`${where}.input must be a non-empty array`);
	for (const modality of model.input) {
		if (modality !== "text" && modality !== "image") fail(`${where}.input has unknown modality ${JSON.stringify(modality)}`);
	}
	const cost = object(model.cost, `${where}.cost`);
	rejectUnknownKeys(cost, COST_KEYS, `${where}.cost`);
	for (const key of COST_RATE_KEYS) finiteNumber(cost[key], `${where}.cost.${key}`);
	if (cost.tiers !== undefined) {
		if (!Array.isArray(cost.tiers)) fail(`${where}.cost.tiers must be an array`);
		for (const [index, rawTier] of cost.tiers.entries()) {
			const tierWhere = `${where}.cost.tiers[${index}]`;
			const tier = object(rawTier, tierWhere);
			rejectUnknownKeys(tier, COST_TIER_KEYS, tierWhere);
			for (const key of COST_RATE_KEYS) finiteNumber(tier[key], `${tierWhere}.${key}`);
			const threshold = finiteNumber(tier.inputTokensAbove, `${tierWhere}.inputTokensAbove`);
			if (!Number.isSafeInteger(threshold) || threshold < 0) {
				fail(`${tierWhere}.inputTokensAbove must be a non-negative safe integer`);
			}
		}
	}
	for (const key of ["contextWindow", "maxTokens"] as const) {
		const value = finiteNumber(model[key], `${where}.${key}`);
		if (!Number.isSafeInteger(value) || value <= 0) fail(`${where}.${key} must be a positive safe integer`);
	}
	if (model.thinkingLevelMap !== undefined) {
		const map = object(model.thinkingLevelMap, `${where}.thinkingLevelMap`);
		rejectUnknownKeys(map, THINKING_KEYS, `${where}.thinkingLevelMap`);
		for (const [key, value] of Object.entries(map)) {
			if (value !== null && typeof value !== "string") fail(`${where}.thinkingLevelMap.${key} must be string or null`);
		}
	}
	if (model.headers !== undefined) {
		for (const [key, value] of Object.entries(object(model.headers, `${where}.headers`))) {
			if (!key || typeof value !== "string") fail(`${where}.headers must map non-empty names to strings`);
		}
	}
	if (model.compat !== undefined) object(model.compat, `${where}.compat`);
	return structuredClone(model);
}

function normalize(raw: unknown): CatalogEntry[] {
	const providers = object(raw, "MODELS");
	const catalog: CatalogEntry[] = [];
	const providerIds = new Set<string>();
	for (const [provider, modelsValue] of Object.entries(providers)) {
		string(provider, "provider id");
		if (providerIds.has(provider)) fail(`duplicate provider ${provider}`);
		providerIds.add(provider);
		const models = object(modelsValue, provider);
		const ids = new Set<string>();
		const normalized: JsonObject[] = [];
		for (const [id, model] of Object.entries(models)) {
			if (ids.has(id)) fail(`duplicate model ${provider}/${id}`);
			ids.add(id);
			normalized.push(validateModel(model, provider, id));
		}
		catalog.push({ provider, models: normalized });
	}
	return catalog;
}

function validateCatalog(catalog: CatalogEntry[]): void {
	const providers = new Set<string>();
	for (const entry of catalog) {
		if (providers.has(entry.provider)) fail(`duplicate provider ${entry.provider}`);
		providers.add(entry.provider);
		const ids = new Set<string>();
		for (const model of entry.models) {
			const id = string(model.id, `${entry.provider} model id`);
			if (ids.has(id)) fail(`duplicate model ${entry.provider}/${id}`);
			ids.add(id);
			validateModel(model, entry.provider, id);
		}
	}
}

async function loadOverrides(path: string): Promise<{ overrides: Override[]; bytes: string }> {
	const bytes = await readFile(path, "utf8");
	const root = object(JSON.parse(bytes), "overrides");
	rejectUnknownKeys(root, new Set(["schemaVersion", "overrides"]), "overrides");
	if (root.schemaVersion !== 1 || !Array.isArray(root.overrides)) fail("overrides must use schemaVersion 1 and an overrides array");
	const overrides = root.overrides.map((raw, index) => {
		const item = object(raw, `overrides[${index}]`);
		rejectUnknownKeys(item, new Set(["provider", "model", "reason", "set", "remove"]), `overrides[${index}]`);
		const override: Override = {
			provider: string(item.provider, `overrides[${index}].provider`),
			model: string(item.model, `overrides[${index}].model`),
			reason: string(item.reason, `overrides[${index}].reason`),
		};
		if (item.set !== undefined) override.set = object(item.set, `overrides[${index}].set`);
		if (item.remove !== undefined) {
			if (!Array.isArray(item.remove) || item.remove.some((key) => typeof key !== "string")) {
				fail(`overrides[${index}].remove must be an array of field names`);
			}
			override.remove = item.remove as string[];
		}
		return override;
	});
	return { overrides, bytes };
}

function applyOverrides(catalog: CatalogEntry[], overrides: Override[]): void {
	const seen = new Set<string>();
	for (const override of overrides) {
		const target = `${override.provider}/${override.model}`;
		if (seen.has(target)) fail(`multiple overrides target ${target}`);
		seen.add(target);
		const provider = catalog.find((entry) => entry.provider === override.provider);
		const model = provider?.models.find((row) => row.id === override.model);
		if (!model) fail(`override target does not exist: ${target}`);
		for (const [key, value] of Object.entries(override.set ?? {})) model[key] = structuredClone(value);
		for (const key of override.remove ?? []) delete model[key];
	}
	validateCatalog(catalog);
}

function sha256(value: string): string {
	return createHash("sha256").update(value).digest("hex");
}

function inventory(catalog: CatalogEntry[]): { providers: number; models: number; byProvider: Record<string, number>; apis: Record<string, number> } {
	const byProvider: Record<string, number> = {};
	const apis: Record<string, number> = {};
	let models = 0;
	for (const entry of catalog) {
		byProvider[entry.provider] = entry.models.length;
		models += entry.models.length;
		for (const model of entry.models) {
			const api = model.api as string;
			apis[api] = (apis[api] ?? 0) + 1;
		}
	}
	return { providers: catalog.length, models, byProvider, apis };
}

async function run(command: string[], cwd?: string): Promise<string> {
	const process = Bun.spawn(command, { cwd, stdout: "pipe", stderr: "inherit" });
	const stdout = await new Response(process.stdout).text();
	if ((await process.exited) !== 0) fail(`command failed: ${command.join(" ")}`);
	return stdout.trim();
}

async function acquire(options: Options): Promise<{ root: string; file: string; revision: string; cleanup?: string }> {
	if (options.source) {
		const source = resolve(options.source);
		if (!existsSync(source)) fail(`source does not exist: ${source}`);
		const isFile = source.endsWith(".ts") || source.endsWith(".js");
		return {
			root: isFile ? dirname(source) : source,
			file: isFile ? source : join(source, options.sourcePath),
			revision: options.revision ?? "local-fixture",
		};
	}
	const temp = await mkdtemp(join(tmpdir(), "pi-model-catalog-"));
	if (options.revision) {
		await run(["git", "init", "--quiet", temp]);
		await run(["git", "-C", temp, "fetch", "--quiet", "--depth", "1", options.repository, options.revision]);
		await run(["git", "-C", temp, "checkout", "--quiet", "FETCH_HEAD"]);
	} else {
		await run(["git", "clone", "--quiet", "--depth", "1", "--branch", options.ref, options.repository, temp]);
	}
	const revision = await run(["git", "rev-parse", "HEAD"], temp);
	return { root: temp, file: join(temp, options.sourcePath), revision, cleanup: temp };
}

async function importModels(file: string): Promise<unknown> {
	if (!existsSync(file)) fail(`generated catalog not found: ${file}`);
	const module = await import(`${pathToFileURL(file).href}?catalog-update=${Date.now()}`);
	if (!("MODELS" in module)) fail(`${file} does not export MODELS`);
	return module.MODELS;
}

function oldInventory(path: string): ReturnType<typeof inventory> | undefined {
	try {
		const raw = JSON.parse(require("node:fs").readFileSync(path, "utf8")) as CatalogEntry[];
		return inventory(raw);
	} catch {
		return undefined;
	}
}

function summary(revision: string, before: ReturnType<typeof inventory> | undefined, after: ReturnType<typeof inventory>, provenanceHash: string): string {
	const providerDelta = before ? after.providers - before.providers : after.providers;
	const modelDelta = before ? after.models - before.models : after.models;
	const signed = (value: number) => (value >= 0 ? `+${value}` : `${value}`);
	return [
		"## Model catalog update",
		"",
		`- source revision: \`${revision}\``,
		`- source catalog SHA-256: \`${provenanceHash}\``,
		`- providers: ${after.providers} (${signed(providerDelta)})`,
		`- models: ${after.models} (${signed(modelDelta)})`,
		`- APIs: ${Object.entries(after.apis).map(([api, count]) => `${api}=${count}`).join(", ")}`,
		"",
		"Generated by `nix run .#update-model-catalog`; schema, duplicate IDs, protocol vocabulary, typed Rust round-trip, protocol replay, and flake checks gate merge.",
		"",
	].join("\n");
}

async function main(): Promise<void> {
	const options = parseArgs(process.argv.slice(2));
	const acquired = await acquire(options);
	try {
		const raw = await importModels(acquired.file);
		const sourceCanonical = JSON.stringify(raw);
		const catalog = normalize(raw);
		const loadedOverrides = await loadOverrides(resolve(options.overrides));
		applyOverrides(catalog, loadedOverrides.overrides);
		const outputPath = resolve(options.output);
		const before = oldInventory(outputPath);
		const output = `${JSON.stringify(catalog, null, "\t")}\n`;
		const after = inventory(catalog);
		const sourceHash = sha256(sourceCanonical);
		const provenance = {
			schemaVersion: 1,
			source: {
				repository: options.source ? options.source : options.repository,
				revision: acquired.revision,
				path: options.sourcePath,
				catalogSha256: sourceHash,
			},
			overrides: {
				path: options.overrides,
				sha256: sha256(loadedOverrides.bytes),
				count: loadedOverrides.overrides.length,
			},
			outputSha256: sha256(output),
			inventory: after,
		};
		await writeFile(outputPath, output);
		await writeFile(resolve(options.provenance), `${JSON.stringify(provenance, null, "\t")}\n`);
		const report = summary(acquired.revision, before, after, sourceHash);
		if (options.summaryOutput) await writeFile(resolve(options.summaryOutput), report);
		console.log(report.trimEnd());
	} finally {
		if (acquired.cleanup) await rm(acquired.cleanup, { recursive: true, force: true });
	}
}

await main().catch((error: unknown) => {
	console.error(error instanceof Error ? error.message : String(error));
	process.exit(1);
});
