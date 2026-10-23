// Runs parity/models.json through upstream's model-resolver.ts (resolveCliModel,
// resolveModelScopeFromModels), one JSON line per case, like examples/models_parity.rs.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const pi = process.env.PI_TS;
if (!pi) throw new Error("PI_TS must point at the upstream checkout");
const { resolveCliModel, resolveModelScopeFromModels } = await import(
	pathToFileURL(join(pi, "packages/coding-agent/src/core/model-resolver.ts")).href
);

const spec = JSON.parse(readFileSync(process.argv[2], "utf8"));
const models = spec.models.map((m) => ({
	...m,
	api: "openai-completions",
	reasoning: false,
	contextWindow: 1000,
	maxTokens: 100,
	baseUrl: "",
	input: ["text"],
}));
const runtime = { getModels: () => models, hasConfiguredAuth: (p) => spec.authed.includes(p) };
const brief = (m) => (m ? { provider: m.provider, id: m.id, name: m.name, reasoning: m.reasoning } : null);

for (const c of spec.cli) {
	const r = resolveCliModel({
		cliProvider: c.provider,
		cliModel: c.model,
		cliThinking: c.thinking,
		inlineApiKey: c.inlineKey,
		modelRuntime: runtime,
	});
	console.log(
		JSON.stringify({
			case: c,
			model: brief(r.model),
			thinking: r.thinkingLevel ?? null,
			warning: r.warning ?? null,
			error: r.error ?? null,
		}),
	);
}
for (const patterns of spec.scope) {
	const r = resolveModelScopeFromModels(patterns, models);
	console.log(
		JSON.stringify({
			case: patterns,
			scoped: r.scopedModels.map((s) => [s.model.provider, s.model.id, s.thinkingLevel ?? null]),
			diagnostics: r.diagnostics.map((d) => [d.code, d.message, d.pattern]),
		}),
	);
}
