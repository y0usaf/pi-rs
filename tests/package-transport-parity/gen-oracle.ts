// Generates tests/package-transport-parity/oracle.json by driving Pi's real
// utils/git.ts parseGitUrl over the source grammar corpus, plus the
// isLocalPath classification Pi's package manager uses to route a source to
// the local-path transport. Run via scripts/package-transport-oracle.
import { readFileSync, writeFileSync } from "node:fs";
import { parseGitUrl } from "../../ref/pi/packages/coding-agent/src/utils/git.ts";
import { isLocalPath } from "../../ref/pi/packages/coding-agent/src/utils/paths.ts";

const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8"));

const out = {
  oracle: "Pi v0.79.0 c5582102",
  gitSources: cases.gitSources.map((c: any) => {
    let parsed: unknown;
    let threw: string | null = null;
    try {
      parsed = parseGitUrl(c.source);
    } catch (e) {
      threw = e instanceof Error ? e.message : String(e);
    }
    return {
      name: c.name,
      source: c.source,
      isLocalPath: isLocalPath(c.source),
      git: threw !== null ? { threw: true, error: threw } : parsed,
    };
  }),
};
writeFileSync(process.argv[3]!, JSON.stringify(out, null, 2));
console.log(`wrote ${process.argv[3]}`);