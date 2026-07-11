#!/usr/bin/env bash
# Generates ARCHITECTURE.md: workspace crate graph + per-crate module structure.
# Regenerate after structural changes; `nix flake check` fails if this file is stale.
# Requires: cargo, jq, cargo-modules (all in the dev shell).
set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-ARCHITECTURE.md}"
META="$(cargo metadata --format-version 1 --no-deps)"

strip_ansi() { sed -e 's/\x1b\[[0-9;]*m//g'; }

{
  echo '<!-- GENERATED FILE — do not edit by hand.'
  echo '     Regenerate: scripts/gen-arch.sh'
  echo '     Freshness is enforced by `nix flake check` (checks.arch-fresh). -->'
  echo
  echo '# Architecture map (generated)'
  echo
  echo 'Structural map of the workspace, extracted from the code. For rationale,'
  echo 'invariants, and dataflow, see DESIGN.md.'
  echo
  echo '## Crate dependency graph'
  echo
  echo 'Internal (workspace-local) dependencies only. `A --> B` means A depends on B.'
  echo
  echo '```mermaid'
  echo 'graph TD'
  jq -r '
    .packages | sort_by(.name)[] | .name as $n |
    [.dependencies[] | select(.path != null) | .name] | unique[] |
    "  \($n | gsub("-"; "_"))[\"\($n)\"] --> \(. | gsub("-"; "_"))[\"\(.)\"]"
  ' <<<"$META"
  echo '```'
  echo
  echo '## Crates'
  echo
  echo '| Crate | Description |'
  echo '|-------|-------------|'
  jq -r '
    .packages | sort_by(.name)[] |
    "| `\(.name)` | \(.description // "_no description in Cargo.toml_") |"
  ' <<<"$META"
  echo
  echo '## Module structure'
  echo
  echo 'Per-crate module/item trees (`cargo modules structure`). Functions are'
  echo 'omitted; types, traits, and module boundaries are the architecture.'
} >"$OUT.tmp"

# One structure tree per crate: the lib target if present, else each bin target.
while IFS=$'\t' read -r pkg kind target; do
  echo "  mapping $pkg ($kind)" >&2
  {
    echo
    echo "### \`$pkg\`"
    echo
    echo '```'
    if [ "$kind" = lib ]; then
      cargo modules structure --package "$pkg" --lib --no-fns 2>/dev/null | strip_ansi
    else
      cargo modules structure --package "$pkg" --bin "$target" --no-fns 2>/dev/null | strip_ansi
    fi
    echo '```'
  } >>"$OUT.tmp"
done < <(jq -r '
  .packages | sort_by(.name)[] | .name as $n |
  (if any(.targets[]; .kind | index("lib")) then
     [$n, "lib", $n]
   else
     (.targets[] | select(.kind | index("bin")) | [$n, "bin", .name])
   end) | @tsv
' <<<"$META")

mv "$OUT.tmp" "$OUT"
echo "wrote $OUT" >&2
