#!/usr/bin/env bash
#
# Overnight parity supervisor for pi-rs.
# Runs `reasonix run` with the hardened parity goal in a loop, relaunching
# every time reasonix exits, so the effort continues all night regardless of
# any single run's completion heuristic. Work persists in the uncommitted tree.
#
# Hardened (review findings):
#   - exponential backoff between relaunches so a fast-failing run can't spin
#     forever minting .parity-metrics-*.json every few seconds;
#   - a hard MAX_ITERS cap as an outer bound;
#   - the resume policy is explicit: each iteration is a FRESH `run --dir`
#     with the full goal prompt (the agent re-orients from the durable
#     audit/state on disk each time). The goal text's "--resume" wording is
#     aspirational; a fresh run is equivalent because every run re-reads the
#     same on-disk state. Set RESUME=1 to pass --resume instead.
#
set -u
REASONIX=/nix/store/312bc8pxxlxdyz18zp72vsk0pwl1ynwv-reasonix-1.24.1/bin/reasonix
DIR=/home/y0usaf/dev/pi-rs
GOAL_FILE="$DIR/.parity-goal.txt"
LOG="$DIR/.parity-supervisor.log"
RUNLOG="$DIR/.parity-run.log"

# Outer bounds for the loop.
MAX_ITERS="${MAX_ITERS:-0}"          # 0 = unlimited; otherwise hard cap
RESUME="${RESUME:-0}"                # 1 = pass --resume, else fresh run
TIMEOUT_HR="${TIMEOUT_HR:-8}"        # hard outer timeout per iteration
BASE_BACKOFF_SEC="${BASE_BACKOFF_SEC:-5}"
MAX_BACKOFF_SEC="${MAX_BACKOFF_SEC:-300}"

if [ ! -f "$GOAL_FILE" ]; then
  echo "$(date +%FT%T) ERROR: goal file missing: $GOAL_FILE" >>"$LOG"
  exit 1
fi

PROMPT="$(cat "$GOAL_FILE")"

iter=0
consec_fail=0
while true; do
  if [ "$MAX_ITERS" -gt 0 ] && [ "$iter" -ge "$MAX_ITERS" ]; then
    echo "$(date +%FT%T) supervisor: reached MAX_ITERS=$MAX_ITERS, stopping" >>"$LOG"
    exit 0
  fi
  iter=$((iter + 1))
  tag="iter-$iter-$(date +%Y%m%d-%H%M%S)"
  metrics="$DIR/.parity-metrics-$tag.json"
  traj="$DIR/.parity-trajectory-$tag.jsonl"
  ts="$(date +%FT%T)"
  echo "$ts supervisor: starting iteration $iter (tag=$tag)" >>"$LOG"

  # Run reasonix headless. Cap each run so a wedged run can't hang forever
  # (reasonix has its own timeouts; this is a hard outer safety net).
  # We use --permission-mode auto for unattended writes and --max-steps 0 for
  # continuous (unbounded) goal looping within the run.
  if [ "$RESUME" = "1" ]; then
    timeout "$((TIMEOUT_HR * 3600))" \
      "$REASONIX" run --resume \
        --dir "$DIR" \
        --permission-mode auto \
        --max-steps 0 \
        --metrics "$metrics" \
        --trajectory "$traj" \
        "$PROMPT" >"$RUNLOG" 2>&1
  else
    timeout "$((TIMEOUT_HR * 3600))" \
      "$REASONIX" run \
        --dir "$DIR" \
        --permission-mode auto \
        --max-steps 0 \
        --metrics "$metrics" \
        --trajectory "$traj" \
        "$PROMPT" >"$RUNLOG" 2>&1
  fi
  code=$?

  echo "$ts supervisor: iteration $iter exited code=$code (elapsed n/a)" >>"$LOG"

  # Backoff: exponential on consecutive failures, reset on success (exit 0).
  if [ "$code" -eq 0 ]; then
    consec_fail=0
    backoff="$BASE_BACKOFF_SEC"
  else
    consec_fail=$((consec_fail + 1))
    backoff=$((BASE_BACKOFF_SEC * (1 << (consec_fail - 1))))
    [ "$backoff" -gt "$MAX_BACKOFF_SEC" ] && backoff="$MAX_BACKOFF_SEC"
  fi

  echo "$ts supervisor: backoff ${backoff}s before next iteration" >>"$LOG"
  sleep "$backoff"
done
