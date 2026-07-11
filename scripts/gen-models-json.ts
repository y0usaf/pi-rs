#!/usr/bin/env bun
// Generate crates/pi-rs-ai/data/models.json from the spec's model catalog
// (ref/pi @ c5582102, pi v0.79.0: packages/ai/src/models.generated.ts).
//
// The catalog is data, never hand code (DESIGN.md, locked `pi-rs-ai` row).
// Output shape: an ordered array of { provider, models: [Model, ...] } —
// arrays make the spec's Record insertion order explicit without relying
// on JSON object key order.
//
// Requires bun (type-stripping import of the spec's .ts). The spec is
// frozen, so regeneration is only needed if the pin moves:
//
//   bun scripts/gen-models-json.ts

import { MODELS } from "../ref/pi/packages/ai/src/models.generated.ts";

const catalog = Object.entries(MODELS).map(([provider, models]) => ({
	provider,
	models: Object.values(models),
}));

const out = new URL("../crates/pi-rs-ai/data/models.json", import.meta.url);
await Bun.write(out, `${JSON.stringify(catalog, null, "\t")}\n`);

const total = catalog.reduce((n, p) => n + p.models.length, 0);
console.log(`wrote ${catalog.length} providers, ${total} models`);
