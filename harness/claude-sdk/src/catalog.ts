/**
 * claude-sdk model catalog — the `supportedModels()` half of pi's
 * modelcatalog.ts amendment (2026-07-20): "the claude-sdk harness can fill
 * this same wire shape from supportedModels() later with no change to the
 * box or the app" (modelcatalog.ts:82).
 *
 * Unlike pi, claude-sdk has no cycling scope to reconstruct (no --models
 * knob, no settings.json, no registry to walk): `Query.supportedModels()`
 * IS the whole menu, straight from a running, authenticated CLI. So this
 * module is a pure, one-shot mapping — no async deps, no precedence ladder.
 *
 * id choice: `ModelInfo.value` (e.g. "sonnet", "claude-opus-4-8") — the
 * exact string `Query.setModel` accepts and `sdkapply.matchModel` verifies
 * against, so a row picked in the app round-trips as a *verified* Tier-1 hot
 * apply. `resolvedModel` is deliberately NOT surfaced as a separate row
 * (dedupe by id, like pi's modelcatalog.ts:224-242).
 *
 * available is always `true`: supportedModels() already comes from a live,
 * authenticated session — there is no per-model auth probe to run, and
 * claiming otherwise would be inventing a signal the SDK doesn't offer.
 *
 * source is always "available" (the CatalogSource type is kept wire-
 * compatible with pi's three-tier union, but claude-sdk has no scope knob to
 * populate "models"/"settings" with — see PARITY.md and the spec's
 * workstream 3 notes; a future HOTLINE_SDK_MODELS scope knob is a possible
 * follow-up, out of scope here).
 */

/** The subset of the SDK's ModelInfo (sdk.d.ts:1205) we read. Structural, so
 * this module never imports the SDK's types and stays unit-testable with
 * plain objects. */
export interface SdkModelInfoLike {
  value?: unknown;
  resolvedModel?: unknown;
  displayName?: unknown;
}

/** Where the catalog's list came from. Kept wire-compatible with pi's
 * three-tier CatalogSource; claude-sdk only ever emits "available". */
export type CatalogSource = "models" | "settings" | "available";

/** One selectable model. `id` is `ModelInfo.value` verbatim — the exact
 * string the app sends back in a set_sdk_config, and the exact string
 * `sdkapply.matchModel` verifies against, so a selected row can actually
 * show as a verified apply. */
export interface CatalogModel {
  id: string;
  label: string;
  available: boolean;
}

/** The harness_catalog notification payload — byte-identical shape to pi's
 * HarnessCatalog (modelcatalog.ts:82-87). */
export interface HarnessCatalog {
  harness: string;
  source: CatalogSource;
  models: CatalogModel[];
  truncated?: boolean;
}

/**
 * Catalog caps. MIRROR internal/app/agentcatalog.go and pi's
 * modelcatalog.ts — the box re-applies its own bounds regardless, so a
 * mismatch would only mean the box silently trimming what we sent. Kept
 * equal so what the harness logs is what the app receives.
 */
export const MAX_CATALOG_MODELS = 64;
export const MAX_CATALOG_MODEL_ID = 64;
export const MAX_CATALOG_LABEL = 40;

/** Single trimmed line, capped at n code points (pi's `line()` rule,
 * modelcatalog.ts:137-140, copied verbatim). */
function line(s: string, n: number): string {
  const flat = s.replace(/\s+/g, " ").trim();
  return [...flat].length <= n ? flat : [...flat].slice(0, n).join("");
}

/** `ModelInfo.value`, or null for a row with no usable id (nothing
 * selectable — dropped, not a blank row). */
export function modelId(model: SdkModelInfoLike): string | null {
  const id = typeof model.value === "string" ? model.value.trim() : "";
  return id === "" ? null : id;
}

/** Display label: `displayName` capped at 40, falling back to the bare id
 * when displayName is absent/blank. */
export function catalogLabel(model: SdkModelInfoLike, id: string): string {
  const name = typeof model.displayName === "string" ? model.displayName.trim() : "";
  return line(name !== "" ? name : id, MAX_CATALOG_LABEL);
}

/**
 * Build the harness_catalog wire shape from `supportedModels()` rows. Pure,
 * synchronous, and never throws: a malformed input (not an array, a row with
 * no id) degrades to dropping that entry — or, for a non-iterable `models`,
 * to an empty catalog — rather than propagating, matching pi's "an empty
 * catalog is a legitimate report, the app keeps its curated fallback"
 * posture (modelcatalog.ts:175-179).
 */
export function buildSdkCatalog(models: SdkModelInfoLike[] | null | undefined): HarnessCatalog {
  const empty: HarnessCatalog = { harness: "claude-sdk", source: "available", models: [] };
  try {
    const out: CatalogModel[] = [];
    const seen = new Set<string>();
    let truncated = false;
    for (const m of models ?? []) {
      const model = (m ?? {}) as SdkModelInfoLike;
      const id = modelId(model);
      if (id === null || id.length > MAX_CATALOG_MODEL_ID || seen.has(id)) continue;
      if (out.length >= MAX_CATALOG_MODELS) {
        truncated = true;
        break;
      }
      seen.add(id);
      out.push({ id, label: catalogLabel(model, id), available: true });
    }
    return { harness: "claude-sdk", source: "available", models: out, ...(truncated ? { truncated: true } : {}) };
  } catch {
    return empty;
  }
}
