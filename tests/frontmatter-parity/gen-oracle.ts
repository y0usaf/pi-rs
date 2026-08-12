// Generates tests/frontmatter-parity/oracle.json by driving Pi's real
// parseFrontmatter (ref/pi/packages/coding-agent/src/utils/frontmatter.ts)
// over edge-case documents. Run via scripts/frontmatter-oracle.
import { readFileSync, writeFileSync } from "node:fs";
import { parseFrontmatter } from "../../ref/pi/packages/coding-agent/src/utils/frontmatter.ts";

const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8")).cases;

const out = {
  oracle: "Pi v0.79.0 c5582102",
  cases: cases.map((c: any) => {
    let result;
    let threw: string | null = null;
    try {
      result = parseFrontmatter(c.content);
    } catch (e) {
      threw = e instanceof Error ? e.message : String(e);
    }
    if (threw !== null) {
      return { name: c.name, content: c.content, threw: true, error: threw };
    }
    return { name: c.name, content: c.content, frontmatter: result.frontmatter, body: result.body };
  }),
};
writeFileSync(process.argv[3]!, JSON.stringify(out, null, 2));
console.log(`wrote ${process.argv[3]}`);
