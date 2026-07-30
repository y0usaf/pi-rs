#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage: spawn-worker.sh \
  --wave <id> --kind <serial|parallel> --slug <slug> --item <PLAN item> \
  --base <commit> --integration <ref> --dependencies <commits|none> \
  --paths <owned paths/globs> --assignment <coherent deliverable>
EOF
  exit 2
}

wave=
kind=
slug=
item=
base=
integration=
dependencies=
paths=
assignment=

while (($#)); do
  [[ $# -ge 2 ]] || usage
  case $1 in
    --wave) wave=$2 ;;
    --kind) kind=$2 ;;
    --slug) slug=$2 ;;
    --item) item=$2 ;;
    --base) base=$2 ;;
    --integration) integration=$2 ;;
    --dependencies) dependencies=$2 ;;
    --paths) paths=$2 ;;
    --assignment) assignment=$2 ;;
    *) usage ;;
  esac
  shift 2
done

for value in wave kind slug item base integration dependencies paths assignment; do
  [[ -n ${!value} ]] || { echo "missing --${value//_/-}" >&2; usage; }
done
[[ $wave =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] || { echo "invalid wave: $wave" >&2; exit 2; }
[[ $kind == serial || $kind == parallel ]] || { echo "invalid kind: $kind" >&2; exit 2; }
[[ $slug =~ ^[a-z0-9][a-z0-9-]*$ ]] || { echo "invalid slug: $slug" >&2; exit 2; }

repo=$(git rev-parse --show-toplevel)
current_head=$(git rev-parse HEAD)
resolved_base=$(git rev-parse --verify "$base^{commit}")
resolved_integration=$(git rev-parse --verify "$integration^{commit}")

[[ -z $(git status --porcelain) ]] || { echo "refusing to spawn from a dirty integration worktree: $repo" >&2; exit 1; }
[[ $current_head == "$resolved_base" ]] || {
  echo "integration worktree HEAD $current_head does not equal base $resolved_base" >&2
  exit 1
}
[[ $resolved_integration == "$resolved_base" ]] || {
  echo "integration ref $integration moved: $resolved_integration != $resolved_base" >&2
  exit 1
}

if [[ $dependencies != none ]]; then
  normalized=${dependencies//,/ }
  for dependency in $normalized; do
    dependency_commit=$(git rev-parse --verify "$dependency^{commit}")
    git merge-base --is-ancestor "$dependency_commit" "$resolved_base" || {
      echo "dependency is not merged into base: $dependency ($dependency_commit)" >&2
      exit 1
    }
  done
fi

[[ -d $repo/ref/pi ]] || { echo "missing pinned oracle: $repo/ref/pi" >&2; exit 1; }
oracle_commit=$(git -C "$repo/ref/pi" rev-parse HEAD)

worktree_root=${PI_PARALLEL_WORKTREE_ROOT:-"$(dirname "$repo")/$(basename "$repo")-parallel"}
state_root=${PI_PARALLEL_STATE_ROOT:-"${XDG_STATE_HOME:-$HOME/.local/state}/pi-rs-parallel"}
worker_branch="parallel/$slug"
worktree="$worktree_root/$slug"
state_dir="$state_root/waves/$wave/$slug"
log="$state_dir/pi.log"
pid_file="$state_dir/pid"
exit_file="$state_dir/exit-code"
meta="$state_dir/meta"
cargo_target="${PI_PARALLEL_CARGO_TARGET_ROOT:-$worktree/target}/parallel-plan"

if git show-ref --verify --quiet "refs/heads/$worker_branch"; then
  echo "branch already exists: $worker_branch" >&2
  exit 1
fi
[[ ! -e $worktree ]] || { echo "worktree path already exists: $worktree" >&2; exit 1; }
[[ ! -e $state_dir ]] || { echo "worker state already exists: $state_dir" >&2; exit 1; }

mkdir -p "$worktree_root" "$state_dir"
git worktree add -b "$worker_branch" "$worktree" "$resolved_base"

cleanup() {
  git worktree remove --force "$worktree" >/dev/null 2>&1 || true
  git branch -D "$worker_branch" >/dev/null 2>&1 || true
  rm -rf "$state_dir"
}
trap cleanup ERR

mkdir -p "$worktree/.pi/skills" "$worktree/ref"
cp -a "$repo/.pi/skills/." "$worktree/.pi/skills/"
ln -s "$repo/ref/pi" "$worktree/ref/pi"

prompt=$(printf '/skill:parallel-plan item=%s\nassignment=%s\nbase=%s\nintegration=%s\ndependencies=%s\npaths=%s' \
  "$item" "$assignment" "$resolved_base" "$integration" "$dependencies" "$paths")

{
  printf 'kind=%s\n' "$kind"
  printf 'wave=%s\n' "$wave"
  printf 'slug=%s\n' "$slug"
  printf 'item=%s\n' "$item"
  printf 'assignment=%s\n' "$assignment"
  printf 'base_commit=%s\n' "$resolved_base"
  printf 'integration=%s\n' "$integration"
  printf 'dependencies=%s\n' "$dependencies"
  printf 'paths=%s\n' "$paths"
  printf 'oracle_commit=%s\n' "$oracle_commit"
  printf 'worker_branch=%s\n' "$worker_branch"
  printf 'worktree=%s\n' "$worktree"
  printf 'log=%s\n' "$log"
  printf 'cargo_target=%s\n' "$cargo_target"
} >"$meta"

(
  cd "$worktree"
  set +e
  pi_args=(
    --approve
    --no-session
    --no-extensions
    --no-prompt-templates
    --no-skills
    --skill "$worktree/.pi/skills/parallel-plan/SKILL.md"
    --thinking "${PI_PARALLEL_THINKING:-high}"
    -p "$prompt"
  )
  if [[ -n ${PI_PARALLEL_MODEL:-} ]]; then
    pi_args=(--model "$PI_PARALLEL_MODEL" "${pi_args[@]}")
  fi
  env PI_SKIP_VERSION_CHECK=1 CARGO_TARGET_DIR="$cargo_target" pi "${pi_args[@]}"
  code=$?
  printf '%s\n' "$code" >"$exit_file.tmp"
  mv "$exit_file.tmp" "$exit_file"
  exit "$code"
) </dev/null >"$log" 2>&1 &
pid=$!
printf '%s\n' "$pid" >"$pid_file"
printf '%s\n' "$wave" >"$state_root/current-wave"
trap - ERR

printf 'spawned %s\n' "$slug"
printf '  kind: %s\n' "$kind"
printf '  wave: %s\n' "$wave"
printf '  pid: %s\n' "$pid"
printf '  branch: %s\n' "$worker_branch"
printf '  worktree: %s\n' "$worktree"
printf '  log: %s\n' "$log"
printf '  cargo target: %s\n' "$cargo_target"
