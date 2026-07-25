/**
 * Pi model catalog (model catalog amendment 2026-07-20) — where the app's model
 * rows actually come from.
 *
 * ## Why the box owns the scope knob
 *
 * The list the app should show is pi's CYCLING scope: the set Ctrl+P walks,
 * which `--models <patterns>` selects. The obvious implementation would be to
 * read that resolved list straight off the live session — and it is not
 * available to us. pi keeps it on `AgentSession._scopedModels` (public getter
 * `scopedModels`), and an extension never receives the AgentSession: neither
 * `ExtensionContext` (ui, mode, cwd, sessionManager, modelRegistry, model,
 * isIdle, …) nor `ExtensionAPI` (setModel, getThinkingLevel, registerTool, …)
 * exposes it, and there is no getScopedModels anywhere on the extension surface
 * as of 0.80.6.
 *
 * So rather than invent a list and call it the scope, we RECONSTRUCT it from
 * the same inputs through the same code:
 *
 *   pi's main.js:  const patterns = parsed.models ?? settings.getEnabledModels();
 *                  const scoped = patterns?.length ? await resolveModelScope(patterns, registry) : [];
 *                  ... and cycleModel() falls back to registry.getAvailable() when scoped is empty.
 *
 *   here:          patterns = HOTLINE_PI_MODELS ?? settings.getEnabledModels();
 *                  same resolveModelScopeWithDiagnostics, same registry;
 *                  same getAvailable() fallback.
 *
 * The one input we cannot read is pi's parsed `--models` argv. The box closes
 * that gap: `hotline up` computes the EFFECTIVE scope (its HOTLINE_PI_MODELS
 * knob, or the operator's `-- --models …` passthrough when that won) and
 * re-exports it as HOTLINE_PI_MODELS into pi's environment, so the string we
 * read here is by construction the string on pi's own launch line. One knob,
 * one list, three surfaces: the launch line, Ctrl+P, and the app.
 *
 * ## Availability
 *
 * `resolveModelScopeWithDiagnostics` resolves patterns against
 * `registry.getAvailable()` — models with auth configured — so a scoped catalog
 * is auth-filtered before we see it, and pi's `_cycleScopedModel` filters again
 * by `hasConfiguredAuth` before cycling. We still compute `available` per entry
 * with `hasConfiguredAuth` rather than hardcoding true: it is the honest field,
 * it is what a future harness filling this same shape from a non-filtered
 * source needs, and if pi ever stops pre-filtering, the flag keeps telling the
 * truth instead of quietly becoming a lie.
 *
 * ## What this is NOT
 *
 * It is not the limit of what the operator can select. The app's free-text
 * escape goes through the hot-apply path, which resolves against
 * `registry.getAll()` — the FULL registry — and fails with `no_api_key` when a
 * model resolves but has no credential. This catalog is the menu; that is the
 * whole kitchen.
 */

import type { Logger } from "@1broseidon/hotline-harness-core/log";

/** The shape we need off a pi Model. Structural, so this module never imports
 * the pi packages and stays unit-testable with plain objects. */
export interface PiModelLike {
  provider?: unknown;
  id?: unknown;
  name?: unknown;
}

/** Where the catalog's list came from. Reported on the wire so the app — and a
 * human reading a box log — can tell a deliberate operator scope from the
 * unscoped default. */
export type CatalogSource = "models" | "settings" | "available";

/** One selectable model. `id` is CANONICAL "provider/id" — the exact string the
 * app sends back in a set_sdk_config, and the exact string the harness restamps
 * as the live identity, so a selected row can actually show as selected. */
export interface CatalogModel {
  id: string;
  label: string;
  available: boolean;
}

/** The harness_catalog notification payload. Harness-agnostic by design: the
 * claude-sdk harness can fill this same shape from supportedModels() later with
 * no change to the box or the app. */
export interface HarnessCatalog {
  harness: string;
  source: CatalogSource;
  models: CatalogModel[];
  truncated?: boolean;
}

/**
 * Catalog caps. These MIRROR internal/app/agentcatalog.go — the box re-applies
 * its own bounds regardless, so a mismatch would only mean the box silently
 * trimming what we sent. Kept equal so what the harness logs is what the app
 * receives.
 */
export const MAX_CATALOG_MODELS = 64;
export const MAX_CATALOG_MODEL_ID = 64;
export const MAX_CATALOG_LABEL = 40;

/** Pattern-list bounds. MIRROR internal/config/pi.go (piModelsMaxPatterns /
 * piModelsMaxPatternLen) — the box normalizes HOTLINE_PI_MODELS with exactly
 * this rule before exporting it, so parsing it again here is idempotent. Kept
 * anyway because this module also runs on a box that set the env var by hand. */
const MAX_PATTERNS = 32;
const MAX_PATTERN_LEN = 64;

/**
 * Split a HOTLINE_PI_MODELS / --models value into patterns. Byte-for-byte the
 * Go ParsePiModels rule, which is itself pi's own arg-parser rule
 * (`args[++i].split(",").map(s => s.trim())`) plus bounds: blanks dropped, an
 * over-long pattern dropped rather than truncated (a truncated glob would match
 * the WRONG models, which is worse than matching none), count capped.
 */
export function parsePiModels(raw: string | undefined): string[] {
  if (!raw) return [];
  const out: string[] = [];
  for (const part of raw.split(",")) {
    const p = part.trim();
    if (p === "" || p.length > MAX_PATTERN_LEN) continue;
    out.push(p);
    if (out.length >= MAX_PATTERNS) break;
  }
  return out;
}

/** Canonical "provider/id" for a pi Model — the same rule index.ts stamps into
 * harness_info, so a catalog row and the live identity are directly comparable.
 * Returns null for a model with no id (nothing selectable). */
export function canonicalId(model: PiModelLike | undefined): string | null {
  if (!model) return null;
  const id = typeof model.id === "string" ? model.id : "";
  if (id === "") return null;
  const provider = typeof model.provider === "string" ? model.provider : "";
  return provider === "" ? id : `${provider}/${id}`;
}

/** Single trimmed line, capped at n code points. */
function line(s: string, n: number): string {
  const flat = s.replace(/\s+/g, " ").trim();
  return [...flat].length <= n ? flat : [...flat].slice(0, n).join("");
}

/** Display label for a model: pi's own `name` when it has one, else the bare id
 * (never the canonical form — the provider is already implied by the harness
 * chip, and repeating it would eat the label budget). */
export function modelLabel(model: PiModelLike): string {
  const name = typeof model.name === "string" ? model.name.trim() : "";
  if (name !== "") return line(name, MAX_CATALOG_LABEL);
  const id = typeof model.id === "string" ? model.id : "";
  return line(id, MAX_CATALOG_LABEL);
}

export interface CatalogDeps {
  /** The effective scope patterns — HOTLINE_PI_MODELS, already parsed. */
  patterns: string[];
  /** pi's SettingsManager.getEnabledModels(): the scope source pi falls back to
   * when no --models flag was passed. Reading it keeps a box configured through
   * pi's own settings.json in agreement with its Ctrl+P. */
  enabledModels: () => string[] | undefined;
  /** pi's resolveModelScopeWithDiagnostics, bound to the live registry. */
  resolveScope: (patterns: string[]) => Promise<{
    scopedModels: Array<{ model: PiModelLike }>;
    diagnostics: Array<{ message?: string }>;
  }>;
  /** registry.getAvailable(): every model with auth configured. */
  getAvailable: () => Promise<PiModelLike[]> | PiModelLike[];
  /** registry.hasConfiguredAuth. */
  hasConfiguredAuth: (model: PiModelLike) => boolean;
  log: Logger;
}

/**
 * Build the catalog, walking pi's own precedence: the effective --models scope,
 * else pi's enabledModels setting, else every model with auth.
 *
 * Never throws. A registry or settings failure is logged and degrades to an
 * empty catalog, and an empty catalog means the app keeps its curated fallback
 * list — so the worst case of a broken registry read is the behavior every box
 * had before this amendment, not a blank model row.
 */
export async function buildCatalog(deps: CatalogDeps): Promise<HarnessCatalog> {
  const empty: HarnessCatalog = { harness: "pi", source: "available", models: [] };
  try {
    let patterns = deps.patterns;
    let source: CatalogSource = "models";
    if (patterns.length === 0) {
      // pi's own fallback: `parsed.models ?? settingsManager.getEnabledModels()`.
      let enabled: string[] | undefined;
      try {
        enabled = deps.enabledModels();
      } catch (err) {
        deps.log.warn(`catalog: enabledModels read failed: ${String(err)}`);
      }
      if (enabled && enabled.length > 0) {
        patterns = parsePiModels(enabled.join(","));
        source = "settings";
      }
    }

    let models: PiModelLike[];
    if (patterns.length > 0) {
      const { scopedModels, diagnostics } = await deps.resolveScope(patterns);
      for (const d of diagnostics) {
        // A pattern matching nothing is the operator's typo, and the ONLY place
        // it can be seen is here — pi prints its own copy to a console no one
        // is reading on a headless box.
        deps.log.warn(`catalog: ${d.message ?? "model scope diagnostic"}`);
      }
      models = scopedModels.map((s) => s.model);
      // A scope that resolved to nothing is NOT an empty menu: pi itself falls
      // back to cycling everything available in that case (cycleModel checks
      // `_scopedModels.length > 0`), so reporting an empty list would advertise
      // a restriction Ctrl+P does not honor.
      if (models.length === 0) {
        deps.log.warn(`catalog: no models matched ${patterns.join(",")}; reporting the available set`);
        models = await deps.getAvailable();
        source = "available";
      }
    } else {
      models = await deps.getAvailable();
      source = "available";
    }

    const out: CatalogModel[] = [];
    const seen = new Set<string>();
    let truncated = false;
    for (const m of models) {
      const id = canonicalId(m);
      if (id === null || id.length > MAX_CATALOG_MODEL_ID || seen.has(id)) continue;
      if (out.length >= MAX_CATALOG_MODELS) {
        truncated = true;
        break;
      }
      seen.add(id);
      let available = false;
      try {
        available = deps.hasConfiguredAuth(m) === true;
      } catch (err) {
        // An auth probe that throws is not evidence of a credential.
        deps.log.warn(`catalog: auth check failed for ${id}: ${String(err)}`);
      }
      out.push({ id, label: modelLabel(m), available });
    }
    return { harness: "pi", source, models: out, ...(truncated ? { truncated: true } : {}) };
  } catch (err) {
    deps.log.warn(`catalog: build failed; the app keeps its fallback list: ${String(err)}`);
    return empty;
  }
}
