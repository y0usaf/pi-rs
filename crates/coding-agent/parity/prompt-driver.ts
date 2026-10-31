import { mkdirSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

const pi = process.env.PI_TS;
if (!pi) throw new Error("PI_TS must point at the upstream checkout");
const src = join(pi, "packages/coding-agent/src");
const load = (p) => import(pathToFileURL(join(src, p)).href);
const { DefaultResourceLoader } = await load("core/resource-loader.ts");
const { SettingsManager } = await load("core/settings-manager.ts");
const { buildSystemPrompt } = await load("core/system-prompt.ts");
const { createAllToolDefinitions } = await load("core/tools/index.ts");

const [casesPath, root] = process.argv.slice(2);
const cases = JSON.parse(readFileSync(casesPath, "utf8"));
for (const c of cases) {
	const base = join(root, c.name);
	for (const [name, text] of Object.entries(c.files ?? {})) {
		const path = join(base, name);
		mkdirSync(dirname(path), { recursive: true });
		writeFileSync(path, text);
	}
	for (const d of c.dirs ?? []) mkdirSync(join(base, d), { recursive: true });
	for (const [link, target] of Object.entries(c.links ?? {})) {
		mkdirSync(dirname(join(base, link)), { recursive: true });
		symlinkSync(target, join(base, link));
	}
	const cwd = join(base, c.cwd ?? "home/proj");
	const agentDir = join(base, "home/.pi/agent");
	mkdirSync(cwd, { recursive: true });
	mkdirSync(agentDir, { recursive: true });
	process.env.HOME = join(base, "home");
	process.env.PI_PACKAGE_DIR = "/pkg";
	const trusted = c.trusted ?? true;
	const settingsManager = SettingsManager.create(cwd, agentDir, { projectTrusted: trusted });
	const loader = new DefaultResourceLoader({
		cwd,
		agentDir,
		settingsManager,
		noExtensions: true,
		disabledBuiltinExtensions: ["mcp", "codemode"],
		noContextFiles: c.noContextFiles,
		noSkills: c.noSkills,
		additionalSkillPaths: c.skills ?? [],
		systemPrompt: c.systemPrompt,
		appendSystemPrompt: c.appendSystemPrompt,
	});
	await loader.reload();
	const defs = createAllToolDefinitions(cwd);
	const tools = (c.tools ?? ["read", "bash", "edit", "write"]).filter((n) => defs[n]);
	const toolSnippets = {};
	const toolGuidelines = {};
	for (const [name, def] of Object.entries(defs)) {
		if (name === "powershell") continue;
		if (def.promptSnippet) toolSnippets[name] = def.promptSnippet;
		toolGuidelines[name] = def.promptGuidelines ?? [];
	}
	const append = loader.getAppendSystemPrompt();
	const prompt = buildSystemPrompt({
		cwd,
		skills: loader.getSkills().skills,
		contextFiles: loader.getAgentsFiles().agentsFiles,
		customPrompt: loader.getSystemPrompt(),
		appendSystemPrompt: append.length > 0 ? append.join("\n\n") : "",
		selectedTools: tools,
		toolSnippets,
		toolGuidelines,
	});
	const diagnostics = loader.getSkills().diagnostics.map((d) => [d.type, d.message, d.path]);
	console.log(JSON.stringify({ name: c.name, prompt: prompt.replaceAll(base, "<base>"), diagnostics: JSON.parse(JSON.stringify(diagnostics).replaceAll(base, "<base>")) }));
}
