/**
 * claude-sdk model catalog (spec workstream 3) — port of the applicable pi
 * run-unit.mjs Part 9 cases onto `buildSdkCatalog`, which maps
 * `Query.supportedModels()` rows directly (no scope/precedence ladder: pi's
 * three tiers collapse to a single "available" source here).
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { buildSdkCatalog, modelId, catalogLabel, MAX_CATALOG_MODELS, MAX_CATALOG_LABEL } from "../dist/catalog.js";

const SONNET = { value: "sonnet", resolvedModel: "claude-sonnet-5", displayName: "Sonnet" };
const OPUS = { value: "claude-opus-4-8", displayName: "Opus 4.8" };
const HAIKU = { value: "claude-haiku-4-5-20251001" }; // no displayName
// A real supportedModels() row carries the context-window suffix in square
// brackets as part of ModelInfo.value — the exact string setModel accepts and
// the catalog surfaces verbatim as a selectable id. The earlier round-trip test
// used only bracket-free fixtures, so it never exercised the id shape that the
// box's set_sdk_config gate actually rejected in the field.
const OPUS_1M = { value: "claude-opus-4-8[1m]", displayName: "Opus 4.8 (1M)" };

test("id = ModelInfo.value verbatim", () => {
  assert.equal(modelId(SONNET), "sonnet");
});
test("a row with no value is not selectable", () => {
  assert.equal(modelId({ displayName: "x" }), null);
  assert.equal(modelId({ value: "" }), null);
});
test("label prefers displayName, and names the generation it resolves to", () => {
  assert.equal(catalogLabel(SONNET, "sonnet"), "Sonnet · sonnet-5");
});
test("an alias label says which model it actually is", () => {
  // "default" and "opus[1m]" both point at opus-5; without the suffix the
  // model row cannot tell the operator which generation it selects.
  const DEFAULT = { value: "default", resolvedModel: "claude-opus-5[1m]", displayName: "Default (recommended)" };
  assert.equal(catalogLabel(DEFAULT, "default"), "Default (recommended) · opus-5[1m]");
  assert.ok([...catalogLabel(DEFAULT, "default")].length <= MAX_CATALOG_LABEL);
});
test("no suffix when resolvedModel is absent, equal to the id, or already shown", () => {
  assert.equal(catalogLabel(OPUS, "claude-opus-4-8"), "Opus 4.8");
  assert.equal(catalogLabel({ value: "sonnet", resolvedModel: "sonnet", displayName: "Sonnet" }, "sonnet"), "Sonnet");
  assert.equal(
    catalogLabel({ value: "x", resolvedModel: "claude-fable-5", displayName: "Fable · fable-5" }, "x"),
    "Fable · fable-5",
  );
});
test("a suffix that would not fit the cap is dropped, never truncated", () => {
  // A truncated model name reads as a different model — worse than none.
  const long = { value: "x", resolvedModel: "claude-some-very-long-model-name-5", displayName: "Some Long Display Name" };
  assert.equal(catalogLabel(long, "x"), "Some Long Display Name");
});
test("label falls back to the id when displayName is absent", () => {
  assert.equal(catalogLabel(HAIKU, "claude-haiku-4-5-20251001"), "claude-haiku-4-5-20251001");
});
test("label flattens whitespace and caps at 40 code points", () => {
  const label = catalogLabel({ value: "x", displayName: "a".repeat(60) }, "x");
  assert.equal(label.length, MAX_CATALOG_LABEL);
});

test("mapping: value -> id, displayName -> label, available always true", () => {
  const cat = buildSdkCatalog([SONNET, OPUS]);
  assert.equal(cat.harness, "claude-sdk");
  assert.equal(cat.source, "available");
  assert.deepEqual(
    cat.models,
    [
      { id: "sonnet", label: "Sonnet · sonnet-5", available: true },
      { id: "claude-opus-4-8", label: "Opus 4.8", available: true },
    ],
  );
});

test("no-id rows are dropped, not blanked", () => {
  const cat = buildSdkCatalog([SONNET, { displayName: "no value" }, OPUS]);
  assert.equal(cat.models.length, 2);
});

test("label falls back to the value when displayName is missing", () => {
  const cat = buildSdkCatalog([HAIKU]);
  assert.equal(cat.models[0].label, "claude-haiku-4-5-20251001");
});

test("dedupe: a repeated value appears once", () => {
  const cat = buildSdkCatalog([SONNET, { value: "sonnet", displayName: "Sonnet (again)" }]);
  assert.equal(cat.models.filter((m) => m.id === "sonnet").length, 1);
});

// MIRROR of internal/app/sdkconfig.go:sdkModelRe — the box-authoritative gate
// every tapped catalog id must clear on its way back in a set_sdk_config,
// BEFORE the harness's matchModel is ever consulted. The original round-trip
// test only checked matchModel (the TS half) and passed while the field failed,
// because the box rejected the bracketed id at this gate first. Keep in lockstep
// with the Go pattern.
const SDK_MODEL_RE = /^[A-Za-z0-9][A-Za-z0-9._:[\]-]{0,63}$/;

test("id round-trips through the box gate and sdkapply.matchModel as a verified apply", async () => {
  const { matchModel } = await import("../dist/sdkapply.js");
  const models = [SONNET, OPUS_1M]; // matchModel reads {value, resolvedModel} off supportedModels() rows
  const cat = buildSdkCatalog(models);
  for (const row of cat.models) {
    // 1. the box's set_sdk_config gate must accept the id the app taps …
    assert.ok(SDK_MODEL_RE.test(row.id), `catalog id ${row.id} must pass the box's sdkModelRe gate`);
    // 2. … and matchModel must then verify it (non-null → Tier-1 verified apply).
    const matched = matchModel(models, row.id);
    assert.notEqual(matched, null, `catalog id ${row.id} must match against supportedModels()`);
  }
});

test("cap: the list is cut at MAX_CATALOG_MODELS and flagged truncated", () => {
  const many = Array.from({ length: MAX_CATALOG_MODELS + 10 }, (_, i) => ({ value: `m${i}`, displayName: `M${i}` }));
  const cat = buildSdkCatalog(many);
  assert.equal(cat.models.length, MAX_CATALOG_MODELS);
  assert.equal(cat.truncated, true);
});
test("cap: an uncut list carries no truncated flag", () => {
  const cat = buildSdkCatalog([SONNET, OPUS]);
  assert.equal(cat.truncated, undefined);
});

test("an over-long id row is dropped", () => {
  const cat = buildSdkCatalog([{ value: "x".repeat(65), displayName: "long" }]);
  assert.equal(cat.models.length, 0);
});

test("empty-in -> empty-out", () => {
  assert.deepEqual(buildSdkCatalog([]), { harness: "claude-sdk", source: "available", models: [] });
});
test("undefined/null input -> empty-out, never throws", () => {
  assert.deepEqual(buildSdkCatalog(undefined), { harness: "claude-sdk", source: "available", models: [] });
  assert.deepEqual(buildSdkCatalog(null), { harness: "claude-sdk", source: "available", models: [] });
});
test("a non-iterable input never throws; degrades to empty", () => {
  assert.doesNotThrow(() => buildSdkCatalog(42));
  assert.deepEqual(buildSdkCatalog(42), { harness: "claude-sdk", source: "available", models: [] });
});
test("garbage rows never throw", () => {
  assert.doesNotThrow(() => buildSdkCatalog([null, undefined, 42, "x", { value: 5 }]));
});
