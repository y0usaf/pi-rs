// Generates tests/skills-parity/oracle.json by driving Pi's real
// core/skills.ts `loadSkillsFromDir` over a temp corpus. Run via
// scripts/skills-oracle. The oracle records per-case the loaded skills
// (name/description/filePath/baseDir/sourceInfo.disableModelInvocation) plus
// diagnostics (type/message/path). Offline checks consume the checked oracle.
import { readFileSync, writeFileSync } from "node:fs";
import { mkdtempSync, mkdirSync, writeFileSync as wfs, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { loadSkillsFromDir } from "../../ref/pi/packages/coding-agent/src/core/skills.ts";

const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8")).cases;

const out = {
  oracle: "Pi v0.79.0 c5582102",
  cases: cases.map((c: any) => {
    const root = mkdtempSync(join(tmpdir(), "pi-skills-"));
    const dir = join(root, c.dir ?? "skills");
    mkdirSync(dir, { recursive: true });
    for (const rel of Object.keys(c.files ?? {})) {
      const full = join(dir, rel);
      mkdirSync(join(full, ".."), { recursive: true });
      let content = c.files[rel];
      // Expand a long-description placeholder so the cases file stays valid
      // JSON while producing a description over MAX_DESCRIPTION_LENGTH.
      content = content.split("$LONG_1100$").join("x".repeat(1100));
      wfs(full, content);
    }

    const result = loadSkillsFromDir({ dir, source: c.source ?? "path" });

    const rel = (p: string) => p.replace(root + "/", "");
    const skillObj = (s: any) => ({
      name: s.name,
      description: s.description,
      filePath: rel(s.filePath),
      baseDir: rel(s.baseDir),
      disableModelInvocation: s.disableModelInvocation,
      sourceInfo: {
        source: s.sourceInfo.source,
        scope: s.sourceInfo.scope,
        origin: s.sourceInfo.origin,
        baseDir: s.sourceInfo.baseDir ? rel(s.sourceInfo.baseDir) : undefined,
      },
    });
    const sorted = result.skills
      .slice()
      .sort((a: any, b: any) => (a.filePath < b.filePath ? -1 : a.filePath > b.filePath ? 1 : 0));

    rmSync(root, { recursive: true, force: true });

    return {
      name: c.name,
      skills: sorted.map(skillObj),
      diagnostics: result.diagnostics
        .slice()
        .sort((a: any, b: any) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0))
        .map((d: any) => ({
          type: d.type,
          message: d.message,
          path: d.path ? rel(d.path) : undefined,
        })),
    };
  }),
};
writeFileSync(process.argv[3]!, JSON.stringify(out, null, 2));
console.log(`wrote ${process.argv[3]}`);