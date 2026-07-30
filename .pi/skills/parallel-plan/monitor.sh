#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 [--wave <id>] [--once | --wait] [--interval <seconds>]" >&2
  exit 2
}

wave=
mode=once
interval=30
while (($#)); do
  case $1 in
    --wave)
      [[ $# -ge 2 ]] || usage
      wave=$2
      shift 2
      ;;
    --once)
      mode=once
      shift
      ;;
    --wait)
      mode=wait
      shift
      ;;
    --interval)
      [[ $# -ge 2 && $2 =~ ^[1-9][0-9]*$ ]] || usage
      interval=$2
      shift 2
      ;;
    *) usage ;;
  esac
done

state_root=${PI_PARALLEL_STATE_ROOT:-"${XDG_STATE_HOME:-$HOME/.local/state}/pi-rs-parallel"}
if [[ -z $wave && -f $state_root/current-wave ]]; then
  wave=$(<"$state_root/current-wave")
fi
[[ -n $wave ]] || { echo "no wave selected and no current wave recorded" >&2; exit 1; }
wave_dir="$state_root/waves/$wave"
[[ -d $wave_dir ]] || { echo "wave state not found: $wave_dir" >&2; exit 1; }

field() {
  local key=$1 file=$2 line
  while IFS= read -r line; do
    if [[ $line == "$key="* ]]; then
      printf '%s' "${line#*=}"
      return 0
    fi
  done <"$file"
  return 1
}

snapshot() {
  local tmp dir slug kind pid worktree branch base integration log state exit_code
  local ahead dirty head log_bytes integration_tip drift
  tmp=$(mktemp "$wave_dir/.status.XXXXXX")
  running=0
  {
    printf 'slug\tkind\tstate\texit\tpid\tbranch\tahead\tdirty\thead\tbase_drift\tlog_bytes\n'
    for dir in "$wave_dir"/*; do
      [[ -d $dir && -f $dir/meta && -f $dir/pid ]] || continue
      slug=${dir##*/}
      kind=$(field kind "$dir/meta" || printf 'unknown')
      pid=$(<"$dir/pid")
      worktree=$(field worktree "$dir/meta" || true)
      branch=$(field worker_branch "$dir/meta" || true)
      base=$(field base_commit "$dir/meta" || true)
      integration=$(field integration "$dir/meta" || true)
      log=$(field log "$dir/meta" || true)

      exit_code=-
      if [[ -f $dir/exit-code ]]; then
        exit_code=$(<"$dir/exit-code")
        if [[ $exit_code == 0 ]]; then state=exited-ok; else state=exited-failed; fi
      elif kill -0 "$pid" 2>/dev/null; then
        state=running
        running=$((running + 1))
      else
        state=exited-unknown
      fi

      ahead=0
      dirty=0
      head=-
      if [[ -n $worktree && -d $worktree ]]; then
        ahead=$(git -C "$worktree" rev-list --count "$base..HEAD" 2>/dev/null || printf '?')
        dirty=$(git -C "$worktree" status --porcelain 2>/dev/null | wc -l)
        head=$(git -C "$worktree" log -1 --pretty=%h 2>/dev/null || printf '-')
      fi

      drift=?
      if [[ -n $worktree && -n $integration ]]; then
        integration_tip=$(git -C "$worktree" rev-parse --verify "$integration^{commit}" 2>/dev/null || true)
        if [[ -n $integration_tip ]]; then
          if [[ $integration_tip == "$base" ]]; then drift=no; else drift=yes; fi
        fi
      fi

      log_bytes=0
      [[ -n $log && -f $log ]] && log_bytes=$(wc -c <"$log")
      printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$slug" "$kind" "$state" "$exit_code" "$pid" "$branch" "$ahead" \
        "$dirty" "$head" "$drift" "$log_bytes"
    done
  } >"$tmp"
  mv "$tmp" "$wave_dir/status.tsv"
  printf 'wave=%s updated=%s\n' "$wave" "$(date --iso-8601=seconds)"
  column -t -s $'\t' "$wave_dir/status.tsv" 2>/dev/null || awk -F '\t' '{ print }' "$wave_dir/status.tsv"
}

while :; do
  snapshot
  [[ $mode == wait && $running -gt 0 ]] || exit 0
  sleep "$interval"
done
