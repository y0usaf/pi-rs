// Generates tests/provider-registry-parity/oracle.json by driving Pi's real
// ModelRegistry.registerProvider validation (coding-agent
// core/model-registry.ts) plus the global custom-API-stream registry
// (ai api-registry.ts), so the port has a differential oracle rather than
// repeating the spec's message strings by hand.
//
// Run via scripts/provider-registry-oracle. Offline normal checks consume the
// checked oracle; opt-in regeneration drives the pinned Pi source.
import { rmSync, writeFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { AuthStorage } from "@earendil-works/pi-coding-agent/core/auth-storage.ts";
import { ModelRegistry } from "@earendil-works/pi-coding-agent/core/model-registry.ts";
import { getApiProvider, getApiProviders } from "@earendil-works/pi-ai";

const out = {
  oracle: "Pi v0.79.0 c5582102",
  cases: [] as unknown[],
};

/** One validation case: register a provider and capture throw/stream registry. */
function runCase(name: string, config: unknown) {
  const dir = mkdtempSync(join(tmpdir(), "pi-provider-"));
  try {
    const auth = AuthStorage.inMemory();
    const registry = ModelRegistry.inMemory(auth);
    let threw: string | null = null;
    try {
      registry.registerProvider(name, config as never);
    } catch (e) {
      threw = e instanceof Error ? e.message : String(e);
    }
    out.cases.push({
      name,
      config: stripFunctions(config),
      threw,
      // The custom API stream handler (if any) keyed by api.
      apiStream: getApiProviders().map((p) => p.api).sort(),
    });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

// A valid streamSimple handler so Pi accepts it (never invoked).
function streamSimple() {}
function freshHandler() {
  return function () {
    /* placeholder */
  };
}

// strip function values so the oracle stays JSON-serializable and
// deterministic across runs (functions are compared for presence only).
function stripFunctions(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(stripFunctions);
  }
  if (value && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      if (typeof v === "function") {
        out[k] = "<function>";
      } else {
        out[k] = stripFunctions(v);
      }
    }
    return out;
  }
  return value;
}

// --- streamSimple / api validation ---
runCase("stream-simple-no-api", { streamSimple });
runCase("stream-simple-empty-api", { api: "", streamSimple });
runCase("stream-simple-blank-api", { api: "   ", streamSimple });
runCase("stream-simple-with-api", { api: "custom-api-a", streamSimple });

// --- models validation ---
const model = (id: string, extra: Record<string, unknown> = {}) => ({
  id,
  name: id,
  reasoning: false,
  input: ["text"],
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  contextWindow: 100000,
  maxTokens: 8000,
  ...extra,
});
runCase("models-no-baseurl", { api: "x", apiKey: "k", models: [model("a")] });
runCase("models-no-credential", { api: "x", baseUrl: "https://x", models: [model("b")] });
runCase("models-with-oauth", { api: "x", baseUrl: "https://x", oauth: { provider: "x" }, models: [model("c")] });
runCase("models-model-no-api", { baseUrl: "https://x", apiKey: "k", models: [model("c")] });
runCase("models-model-own-api", { baseUrl: "https://x", apiKey: "k", models: [model("d", { api: "x" })] });
runCase("models-provider-api", { api: "x", baseUrl: "https://x", apiKey: "k", models: [model("e")] });

// --- custom stream simple cross-provider and unregister ---
runCase("stream-two-providers-same-source", { api: "custom-api-a", streamSimple });
runCase("stream-streamsimple-dispatches-before-rust", { api: "custom-api-b", streamSimple });

writeFileSync(process.argv[2]!, JSON.stringify(out, null, 2));
console.log(`wrote ${process.argv[2]}`);
