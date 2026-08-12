// Generates tests/footer-git-parity/oracle.json by driving Pi's real
// `packages/coding-agent/src/core/footer-data-provider.ts` over real `.git`
// fixtures built on disk with system `git`. Each case constructs a directory
// tree (a normal repo, a worktree via a `.git` file, a detached-HEAD repo, a
// plain directory, a nested cwd) and records what `FooterDataProvider.getGitBranch()`
// reports, plus the async refresh result after the branch is switched.
//
// Run via scripts/footer-git-oracle.
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { FooterDataProvider } from "../../ref/pi/packages/coding-agent/src/core/footer-data-provider.ts";

function git(args: string[], cwd: string) {
  execFileSync("git", args, { cwd, stdio: "ignore" });
}

async function main() {
  const cases: Array<Record<string, unknown>> = [];
  const root = mkdtempSync(join(tmpdir(), "pi-fg-"));

  function caseDir(name: string): string {
    return join(root, name);
  }

  // 1. Normal git repo, branch "main".
  {
    const dir = caseDir("normal");
    mkdirSync(dir, { recursive: true });
    git(["init", "-q", "-b", "main"], dir);
    writeFileSync(join(dir, "a.txt"), "a");
    git(["add", "a.txt"], dir);
    git(["commit", "-q", "-m", "init"], dir);
    const provider = new FooterDataProvider(dir);
    cases.push({ name: "normal-repo", branch: provider.getGitBranch() });
    provider.dispose();
  }

  // 2. Branch switch triggers the async refresh path.
  {
    const dir = caseDir("switch");
    mkdirSync(dir, { recursive: true });
    git(["init", "-q", "-b", "main"], dir);
    writeFileSync(join(dir, "a.txt"), "a");
    git(["add", "a.txt"], dir);
    git(["commit", "-q", "-m", "init"], dir);
    const provider = new FooterDataProvider(dir);
    const before = provider.getGitBranch();
    git(["checkout", "-q", "-b", "feature"], dir);
    const refreshed = await new Promise<string | null | undefined>((resolvePromise) => {
      provider.onBranchChange(() => resolvePromise(provider.getGitBranch()));
      setTimeout(() => resolvePromise("timeout"), 3000);
    });
    cases.push({ name: "switch-branch", before, refreshed });
    provider.dispose();
  }

  // 3. Detached HEAD.
  {
    const dir = caseDir("detached");
    mkdirSync(dir, { recursive: true });
    git(["init", "-q", "-b", "main"], dir);
    writeFileSync(join(dir, "a.txt"), "a");
    git(["add", "a.txt"], dir);
    git(["commit", "-q", "-m", "init"], dir);
    git(["checkout", "-q", "--detach"], dir);
    const provider = new FooterDataProvider(dir);
    cases.push({ name: "detached-head", branch: provider.getGitBranch() });
    provider.dispose();
  }

  // 4. Plain directory (not a repo).
  {
    const dir = caseDir("plain");
    mkdirSync(dir, { recursive: true });
    const provider = new FooterDataProvider(dir);
    cases.push({ name: "plain-dir", branch: provider.getGitBranch() });
    provider.dispose();
  }

  // 5. Nested cwd inside a repo — discovery walks up.
  {
    const dir = caseDir("nested");
    mkdirSync(dir, { recursive: true });
    git(["init", "-q", "-b", "main"], dir);
    writeFileSync(join(dir, "a.txt"), "a");
    git(["add", "a.txt"], dir);
    git(["commit", "-q", "-m", "init"], dir);
    const sub = join(dir, "sub/dir");
    mkdirSync(sub, { recursive: true });
    const provider = new FooterDataProvider(sub);
    cases.push({ name: "nested-cwd", branch: provider.getGitBranch() });
    provider.dispose();
  }

  // 6. Worktree — `.git` is a file pointing at the real git dir.
  {
    const mainDir = caseDir("worktree-main");
    mkdirSync(mainDir, { recursive: true });
    git(["init", "-q", "-b", "main"], mainDir);
    writeFileSync(join(mainDir, "a.txt"), "a");
    git(["add", "a.txt"], mainDir);
    git(["commit", "-q", "-m", "init"], mainDir);
    git(["worktree", "add", "-q", "-b", "wt", join(root, "worktree-wt")], mainDir);
    const wtDir = join(root, "worktree-wt");
    const provider = new FooterDataProvider(wtDir);
    cases.push({ name: "worktree", branch: provider.getGitBranch() });
    provider.dispose();
  }

  const out = {
    oracle: "Pi v0.79.0 c5582102",
    cases,
  };
  writeFileSync(process.argv[2]!, JSON.stringify(out, null, 2));
  rmSync(root, { recursive: true, force: true });
  console.log(`wrote ${process.argv[2]}`);
}

void main();