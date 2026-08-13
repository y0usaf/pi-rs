// Generates tests/prompt-parity/oracle.json by driving Pi's real
// prompt-templates.ts pure functions (parseCommandArgs, substituteArgs,
// expandPromptTemplate) over edge-case inputs. Run via scripts/prompt-oracle.
import { readFileSync, writeFileSync } from "node:fs";
import {
  parseCommandArgs,
  substituteArgs,
  expandPromptTemplate,
} from "../../ref/pi/packages/coding-agent/src/core/prompt-templates.ts";

const cases = JSON.parse(readFileSync(process.argv[2]!, "utf8"));

const out = {
  oracle: "Pi v0.79.0 c5582102",
  parseArgs: cases.parseArgs.map((c: any) => {
    let result: unknown;
    let threw: string | null = null;
    try {
      result = parseCommandArgs(c.input);
    } catch (e) {
      threw = e instanceof Error ? e.message : String(e);
    }
    if (threw !== null) return { name: c.name, input: c.input, threw: true, error: threw };
    return { name: c.name, input: c.input, args: result };
  }),
  substitute: cases.substitute.map((c: any) => {
    let result: unknown;
    let threw: string | null = null;
    try {
      result = substituteArgs(c.content, c.args);
    } catch (e) {
      threw = e instanceof Error ? e.message : String(e);
    }
    if (threw !== null)
      return { name: c.name, content: c.content, args: c.args, threw: true, error: threw };
    return { name: c.name, content: c.content, args: c.args, result };
  }),
  expand: cases.expand.map((c: any) => {
    let result: unknown;
    let threw: string | null = null;
    try {
      result = expandPromptTemplate(c.text, c.templates);
    } catch (e) {
      threw = e instanceof Error ? e.message : String(e);
    }
    if (threw !== null)
      return { name: c.name, text: c.text, threw: true, error: threw };
    return { name: c.name, text: c.text, result };
  }),
};
writeFileSync(process.argv[3]!, JSON.stringify(out, null, 2));
console.log(`wrote ${process.argv[3]}`);