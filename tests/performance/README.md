# Release performance harness v1

`reference-v1.json` fixes the sample counts, warmups, idle settle period, and
canonical experience revision for PLAN 0.2. Run the harness in release mode:

```console
CARGO_TARGET_DIR="$PWD/target/parallel-plan" \
  cargo run --release -p pi-rs-app --bin performance-baseline -- \
  --config tests/performance/reference-v1.json --output result.json
```

The command refuses a debug build. Its only stdout/file payload is versioned,
machine-readable JSON. It records source/fixture revisions, machine details,
sample counts, warm/cold conditions, and distributions for:

- process startup → first input-ready frame;
- settled idle RSS;
- injected input → flushed frame acknowledgement;
- unchanged and one-row-changed retained rendering + changed-frame throughput;
- no-op and three-action Lua dispatch + copied snapshot bytes;
- a local deterministic timer-effect round trip.

Startup samples follow explicit warmup processes. Input latency includes the
cross-process pipe handoff used to inject input and acknowledge the flush. RSS
uses Linux `/proc/<pid>/status`, so reference acceptance runs on Linux. Render,
Lua, and effect samples run in-process against release mechanisms and do not
contact a network.

`performance-baseline --self-test` is a debug-safe structural check used by the
normal Rust suite. Normal checks do not execute the timed benchmark and never
execute Node, Bun, TypeScript, or Pi. Numeric acceptance baselines and budgets
live only in `DESIGN.md`; result files are diagnostic run artifacts and are not
committed.
