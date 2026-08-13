// Generates tests/prompt-loader-parity/oracle.json by driving Pi's real
// core/prompt-templates.ts `loadPromptTemplates` over a temp corpus.
// Run via scripts/prompt-loader-oracle. The oracle records, per case, the
// loaded templates (name/description/argumentHint/content/sourceInfo.filePath
// baseDir/scope/source/origin) plus diagnostics none. Offline checks consume
// the checked oracle; regeneration drives the pinned Pi source.
import { readFileSync, writeFileSync } from "node:fs";
import { mkdtempSync, mkdirSync, writeFileSync as wfs, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { loadPromptTemplates } from "../../ref/pi/packages/coding-agent/src/core/prompt-templates.ts";

const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8")).cases;

const out = {
  oracle: "Pi v0.79.0 c5582102",
  cases: cases.map((c: any) => {
    const root = mkdtempSync(join(tmpdir(), "pi-prompt-loader-"));
    const cwd = join(root, "cwd");
    const agentDir = join(root, "agent");
    mkdirSync(cwd, { recursive: true });
    mkdirSync(agentDir, { recursive: true });
    // Seed a stable project and global prompts dir.
    mkdirSync(join(cwd, ".pi", "prompts"), { recursive: true });
    mkdirSync(join(agentDir, "prompts"), { recursive: true });
    wfs(
      join(cwd, ".pi", "prompts", "project-md.md"),
      "---\ndescription: Project template description\n---\nProject body $1",
    );
    wfs(join(agentDir, "prompts", "global-md.md"), "---\nargument-hint: <arg>\n---\nGlobal body", );

    // Case-specific prompt paths (absolute resolved against root).
    const promptPaths = (c.promptPaths ?? []).map((p: string) => join(root, p));
    // Stage any corpus files the case declares.
    for (const rel of c.files ?? []) {
      const full = join(root, rel);
      mkdirSync(join(full, ".."), { recursive: true });
      wfs(full, c.fileContents?.[rel] ?? "---\ndescription: staged\n---\nStaged body");
    }

    const templates = loadPromptTemplates({
      cwd,
      agentDir,
      promptPaths,
      includeDefaults: c.includeDefaults ?? true,
    });

    // Serialize with path normalization relative to the temp seed root.
    // Templates are sorted by filePath: Pi's loadTemplatesFromDir uses
    // nondeterministic readdir order, so sorting both the oracle record and
    // the port's comparison keeps the differential deterministic.
    const rel = (p: string) => {
      const s = p.replace(root + "/", "");
      return s;
    };
    const serialized = templates
      .sort((a, b) => (a.filePath < b.filePath ? -1 : a.filePath > b.filePath ? 1 : 0))
      .map((t) => ({
        name: t.name,
        description: t.description,
        argumentHint: t.argumentHint,
        content: t.content,
        filePath: rel(t.filePath),
        sourceInfo: {
          source: t.sourceInfo.source,
          scope: t.sourceInfo.scope,
          origin: t.sourceInfo.origin,
          baseDir: t.sourceInfo.baseDir ? rel(t.sourceInfo.baseDir) : undefined,
        },
      }));

    rmSync(root, { recursive: true, force: true });

    return {
      name: c.name,
      includeDefaults: c.includeDefaults ?? true,
      promptPaths: promptPaths.map((p) => rel(p)),
      templates: serialized,
    };
  }),
};
writeFileSync(process.argv[3]!, JSON.stringify(out, null, 2));
console.log(`wrote ${process.argv[3]}`);