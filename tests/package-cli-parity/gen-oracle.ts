// PLAN 9.7 package CLI differential oracle: drive Pi's real
// handlePackageCommand over the parse/help/error surface only — cases that
// return before any settings/trust/network/package-manager side effect — and
// record the observable stdout / stderr / exitCode / handled so pi-rs's
// package CLI can be matched byte-for-byte.
//
// Run via scripts/package-cli-oracle. Offline checks consume the checked
// oracle; opt-in regeneration drives the pinned Pi source.
import { handlePackageCommand } from "../../ref/pi/packages/coding-agent/src/package-manager-cli.ts";
import { writeFileSync } from "node:fs";

function stripAnsi(s: string): string {
  // eslint-disable-next-line no-control-regex
  return s.replace(/\u001b\[[0-9;]*m/g, "");
}

type Json = any;

// Only cases that return before settings/trust/network/package-manager work
// (help, non-command, missing source, invalid option/argument, conflicting
// options, missing option value). Parse reality is pinned through these
// observable, hermetic outcomes.
const CASES: Array<{ name: string; argv: string[] }> = [
  // help
  { name: "install-help", argv: ["install", "--help"] },
  { name: "install-help-h", argv: ["install", "-h"] },
  { name: "remove-help", argv: ["remove", "--help"] },
  { name: "uninstall-help", argv: ["uninstall", "--help"] },
  { name: "update-help", argv: ["update", "--help"] },
  { name: "list-help", argv: ["list", "--help"] },
  // not a package command -> handlePackageCommand returns false (no output)
  { name: "not-a-command", argv: ["echo", "hi"] },
  { name: "garbage-command", argv: ["bogus"] },
  // missing source
  { name: "install-missing-source", argv: ["install"] },
  { name: "remove-missing-source", argv: ["remove"] },
  // invalid option for the command
  { name: "list-local-invalid", argv: ["list", "--local"] },
  { name: "list-self-invalid", argv: ["list", "--self"] },
  { name: "install-self-invalid", argv: ["install", "x", "--self"] },
  { name: "remove-force-invalid", argv: ["remove", "x", "--force"] },
  { name: "list-unknown-flag", argv: ["list", "--nope"] },
  // invalid argument (too many positional)
  { name: "install-two-sources", argv: ["install", "a", "b"] },
  { name: "remove-two-sources", argv: ["remove", "a", "b"] },
  // conflicting options
  // (note: `update --self --extensions` is NOT a conflict in Pi — it maps to
  // target "all" and proceeds to network work, so it is intentionally absent
  // from this hermetic parse-surface oracle.)
  { name: "update-self-and-pos", argv: ["update", "foo", "--self"] },
  { name: "update-ext-and-pos", argv: ["update", "foo", "--extensions"] },
  { name: "update-two-extension", argv: ["update", "--extension", "a", "--extension", "b"] },
  { name: "update-extension-and-self", argv: ["update", "--extension", "a", "--self"] },
  { name: "install-extension-invalid", argv: ["install", "x", "--extension", "a"] },
  // missing option value
  { name: "update-extension-missing-value", argv: ["update", "--extension"] },
  { name: "remove-extension-missing", argv: ["remove", "x", "--extension"] },
];

async function main(): Promise<void> {
  const out: Json = { oracle: "Pi v0.79.0 c5582102", cases: [] as Json[] };

  for (const c of CASES) {
    let stdout = "";
    const stderrLines: string[] = [];
    const origLog = console.log;
    const origError = console.error;
    const origWarn = console.warn;
    const writeOut = (chunk: any) => {
      stdout += String(chunk);
    };
    const writeErr = (chunk: any) => {
      stderrLines.push(String(chunk));
    };
    console.log = function (...args: any[]) {
      writeOut(args.join(" ") + "\n");
    } as typeof console.log;
    console.error = function (...args: any[]) {
      writeErr(args.join(" ") + "\n");
    } as typeof console.error;
    console.warn = function (...args: any[]) {
      writeErr(args.join(" ") + "\n");
    } as typeof console.warn;

    process.exitCode = 0;
    let handled: boolean;
    let threw: string | null = null;
    try {
      handled = await handlePackageCommand(c.argv);
    } catch (e: any) {
      handled = false;
      threw = e instanceof Error ? e.message : String(e);
    }
    const code = process.exitCode;
    console.log = origLog;
    console.error = origError;
    console.warn = origWarn;

    out.cases.push({
      name: c.name,
      argv: c.argv,
      handled,
      exitCode: code,
      stdout: stripAnsi(stdout),
      stderr: stripAnsi(stderrLines.join("")),
      threw,
    });
  }

  writeFileSync(process.argv[2]!, JSON.stringify(out, null, 2) + "\n");
  console.log(`wrote ${process.argv[2]}`);
}
main().catch((e) => {
  throw e;
});