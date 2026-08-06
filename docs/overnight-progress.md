# Overnight progress — pi-rs faithful parity + Prime Agent

## Status (updated at every phase boundary / commit)

- **Baseline / gate:** established 2025-08-06. `nix flake check` was **red** on
  `construction-inventory` (stale theme/anchor coverage) and `final-parity-audit`
  (stale evidence/classification). Additionally `workspace-clippy` was red
  (multiple lints across `pi-rs-ai`, `pi-rs-ai-auth`, `pi-rs-host`, `pi-rs-app`)
  and `arch-fresh` was red (stale generated architecture map). All have been
  fixed and committed. **`nix flake check --no-write-lock-file` now exits 0,
  all 10 checks green** (workspace-test, workspace-clippy, arch-fresh,
  bare-boot, model-catalog-update, construction-inventory,
  external-extension-inventory, extension-parity, dogfood-fixtures,
  final-parity-audit).
- **Current phase:** P0 — faithful parity closure (PLAN.md).
- **Current item:** A.1 — compact exact UI evidence (tests/ui-parity oracles).
- **Next action:** implement compact oracle format in `pi-rs-tui::ui_harness`
  (palette + runs + frame deltas, version 1), wire `ui-diff` to encode/decode,
  convert the 26 oracles, update `scripts/ui-diff` + `FINAL_PARITY_AUDIT.md`
  evidence paths, add a `ui-parity` flake check so retained checkpoints compare
  continuously, verify 95% byte reduction + round-trip + negative controls.
- **Note:** the flake gate being green does **not** mean P0/PARITY is closed:
  every `PLAN.md` item (A.1–A.3, 9.2–9.11, 8, 10, 11) is still unchecked. P0
  closes only when the final parity/ablation audit is green AND all PLAN items
  are checked off with gate evidence in the same commit.

## Done

- **Gate restored to green** (this session, `main` b504d88/cfc49ff/f27940f,
  pushed):
  - `workspace-clippy` red -> green: collapsed if/match in
    `pi-rs-ai-auth::github_copilot`, `pi-rs-ai::openai_responses`; `.ok()` assertions
    in `azure_openai_responses` tests; De Morgan bool +
    `result_large_err` allow (externally-fixed tungstenite callback) +
    redundant-closure in `pi-rs-ai` integration tests; `#![allow(clippy::unwrap_used)]`
    in `pi-rs-host::config` unit tests (established pattern); removed
    needless struct updates in `pi-rs-host::settings` + `pi-rs-app::main`.
    Gate: `nix build .#checks.x86_64-linux.workspace-clippy` exits 0.
  - `arch-fresh` red -> green: regenerated `ARCHITECTURE.md`
    (`nix develop --command bash scripts/gen-arch.sh`). Gate: `nix build
    .#checks.x86_64-linux.arch-fresh` exits 0.
  - Committed the prior session's manifest/audit reconciliation
    (`FINAL_PARITY_AUDIT.md`, both `manifest.json`) that closes
    `construction-inventory` and `final-parity-audit`.
  - Full gate quoted below (exit 0).
- Baseline `nix flake check` run: RED (see findings).
- Fixed `construction-inventory` (manifest drift vs code):
  - `resource.theme-dark/light` coverage `.json` -> `.lua`.
  - `source.coding-agent-frontend` declarations += `extension-event-fold-parity`.
  - `slot.widgets` anchors => current `widget_above`/`render_ui_slot` text.
  - 3 dogfood rows' PLAN.md anchors => current plan text.
  - `scripts/construction-inventory --check` + `test_checker.py` green.
- Fixed `final-parity-audit` (stale evidence/classification):
  - differences 3 & 6 evidence needles => current PLAN.md text.
  - `ai.protocol-tail` needle => `certificate external-account ADC`.
  - 8 group evidence anchors updated to current PLAN.md/DESIGN.md text.
  - `coding.assembly` reclassified owner PLAN 9.1b -> PLAN 11 with honest
    rationale (generic assembly implemented; final side-by-side CLI gate open).
  - Regenerated `FINAL_PARITY_AUDIT.md`; `--check` + `--self-test` green.

## Blockers

- None currently. (If a parity check is red and cannot be fixed honestly:
  stop that line, record here + PLAN.md, move to independent work.)

## Decisions needing human review

- None so far.

## Gate evidence (quoted nix commands)

- **GREEN (this run):** `nix flake check --no-write-lock-file` -> exit 0,
  `all checks passed!` (10 checks). Commits b504d88 (clippy), cfc49ff (arch),
  f27940f (inventories), pushed to origin/main.
  Specific reds fixed (each now exits 0):
  `nix build .#checks.x86_64-linux.workspace-clippy` -> exit 0;
  `nix build .#checks.x86_64-linux.arch-fresh` -> exit 0;
  `nix build .#checks.x86_64-linux.construction-inventory` -> exit 0;
  `nix build .#checks.x86_64-linux.final-parity-audit` -> exit 0.
  Offline: `python3 scripts/construction-inventory --check` -> 0;
  `python3 scripts/final-parity-audit --check` -> 0;
  `python3 scripts/final-parity-audit --self-test` -> 0;
  `python3 scripts/extension-inventory --check` -> 0;
  `python3 scripts/dogfood-oracle --check` -> 0.
- Baseline failure (pre-work):
  `nix flake check --no-write-lock-file` -> construction-inventory FAIL;
  `python3 scripts/construction-inventory --check` -> theme coverage drift.
- Post-fix offline checks (each exited 0):
  `python3 scripts/construction-inventory --check`
  `python3 tests/construction-inventory/test_checker.py`
  `python3 scripts/final-parity-audit --check`
  `python3 scripts/final-parity-audit --self-test`
  `python3 scripts/extension-inventory --check`
  `python3 scripts/dogfood-oracle --check`
  `python3 tests/dogfood-suite/test_contract.py`
