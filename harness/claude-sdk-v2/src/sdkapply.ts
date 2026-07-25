/**
 * SDK hot apply (SDK hot-model amendment 2026-07-19; effort hot amendment
 * 2026-07-19): the box forwards a hot-capable set_sdk_config as a
 * notifications/hotline/sdk_apply notification over the run child's stdio; this
 * handler applies model and/or effort to the LIVE SDK session — no restart, the
 * wire never drops — and answers on notifications/hotline/sdk_apply_result.
 *
 *   - model → Query.setModel (validated against the CLI's supportedModels()
 *     BEFORE the control is ever issued, with dated/context-suffix tolerance so
 *     a bare id like "claude-opus-4-8" matches the catalog's
 *     "claude-opus-4-8[1m]" row — the bug that made the running model look
 *     "unavailable").
 *   - effort → Query.applyFlagSettings({effortLevel}) for a symbolic level, or
 *     Query.setMaxThinkingTokens(n) for a raw budget (effortToSdkApply, the
 *     same mapping the boot Options use — no drift).
 *
 * Validate before, persist after: an invalid effort is refused without ever
 * touching the session, and the box persists to .env only after our ok lands.
 * Model uses a two-tier guard (unverified-model amendment 2026-07-19): a
 * catalog hit applies + restamps the canonical id; a well-formed id absent from
 * the (under-enumerating) catalog applies ANYWAY carrying unverified:true — the
 * API, not the catalog, is the truth, and its rejection surfaces recoverably on
 * the next turn.
 */

import type { Logger } from "@1broseidon/hotline-harness-core/log";
import { effortToSdkApply, type SdkEffortLevel } from "./options.js";

/** The slice of the SDK Query handle the hot-apply path touches. */
export interface SdkApplyQuery {
  supportedModels(): Promise<Array<{ value: string; resolvedModel?: string }>>;
  setModel(model?: string): Promise<void>;
  applyFlagSettings(settings: { effortLevel?: SdkEffortLevel | null }): Promise<void>;
  setMaxThinkingTokens(maxThinkingTokens: number | null): Promise<void>;
}

/**
 * The live session this apply is bound to.
 *
 * An apply is a sequence of awaits — supportedModels, then two or three
 * control calls — and the session underneath can die at any of them: the query
 * stream ends and runAgent starts a fresh one, or the hotline child respawns.
 * Holding only the Query handle (as this used to) meant an apply could capture
 * query A, await, and then push a model onto a DEAD A while reporting success
 * for the live B — restamping harness_info and persisting .env for a session
 * that never saw the change.
 *
 * `generation` is the fence. It advances whenever the query handle changes or
 * the child becomes ready, so re-reading it after every await answers exactly
 * one question: is the thing I captured still the thing that is running?
 */
export interface SdkApplySession {
  query: SdkApplyQuery;
  generation: number;
}

/** The sdk_apply_result wire payload (internal box↔harness stdio contract). */
export interface SdkApplyResult {
  rid: string;
  ok: boolean;
  /** On ok: the applied model id, present iff the request carried model ("" for clear). */
  model?: string;
  /** On ok: the applied effort, present iff the request carried effort ("" for clear). */
  effort?: string;
  /**
   * On ok: true iff the applied model was NOT in the CLI's supportedModels()
   * catalog but was syntactically valid (Tier 2). The catalog under-enumerates
   * live-valid ids, so we apply anyway; a truly bogus id surfaces as an API
   * error on the next turn (recoverable by flipping back). Omitted (verified)
   * for catalog hits and clears, so old boxes' frames are byte-identical.
   */
  unverified?: boolean;
  code?: "unknown_model" | "invalid_effort" | "apply_failed" | "no_session";
  /** Single line, ≤ 200 chars. */
  detail?: string;
}

/** What onApplied conveys: only the fields the request actually changed. A
 * present field mirrors into the harness env + harness_info restamp; "" = the
 * field was cleared back to the SDK default. */
export interface SdkApplied {
  /** present iff the request carried model ("" = cleared). */
  model?: string;
  /** canonical id the applied model resolves to (undefined for clear/unknown). */
  resolvedModel?: string;
  /** present iff the request carried effort ("" = cleared). */
  effort?: string;
}

export interface SdkApplyDeps {
  /** The live session (query handle + generation fence), or null while no SDK
   * session is running. Re-read after every await — see SdkApplySession. */
  getSession: () => SdkApplySession | null;
  /** Sends an sdk_apply_result notification back to the run child. */
  notifyResult: (result: SdkApplyResult) => void;
  /**
   * Called after a successful apply and BEFORE the ok result is notified: the
   * harness updates its local env mirrors (HOTLINE_SDK_MODEL / HOTLINE_SDK_EFFORT
   * for any harness-spawned respawned child) and restamps harness_info, so the
   * box's live identity — and therefore its no-op check — tells the truth before
   * the box answers the app. On a partial failure (model applied, effort threw)
   * this still fires for the part that landed, keeping harness_info honest.
   */
  onApplied: (applied: SdkApplied) => void;
  log: Logger;
}

/** First line of a message, capped at 200 chars (the result detail cap). */
export function firstLine(s: string): string {
  const line = s.split("\n", 1)[0] ?? "";
  return line.length > 200 ? line.slice(0, 200) : line;
}

/**
 * The characters that end a model-id component. A prefix match is only
 * meaningful when it stops at one of these — "claude-opus-4-8" genuinely names
 * the model the catalog lists as "claude-opus-4-8[1m]" or
 * "claude-haiku-4-5-20251001", because the suffix is a separate component.
 */
const MODEL_ID_DELIMITERS = new Set(["-", "_", ".", ":", "[", "(", "@", "/"]);

/**
 * Does `candidate` extend `requested` at a component boundary?
 *
 * Bare `startsWith` (what this used to be) classified any prefix as a catalog
 * hit, so a one-character request like "c" matched "claude-opus-4-8" and was
 * reported as a VERIFIED apply of that model. That inverted the two-tier guard
 * exactly where it matters: junk sailed through as verified while the caution
 * that exists for unrecognized ids was suppressed. Requiring a delimiter keeps
 * every real alias case working and makes a partial id a Tier 2 miss, which is
 * the honest answer.
 */
export function idExtendsAtBoundary(candidate: string, requested: string): boolean {
  if (candidate.length <= requested.length) return false;
  if (!candidate.startsWith(requested)) return false;
  return MODEL_ID_DELIMITERS.has(candidate[requested.length]);
}

/**
 * Match a requested model id against the CLI's supportedModels() rows,
 * returning the canonical resolved id to restamp, or null when nothing matches
 * (→ Tier 2, applied unverified). Exact match on a row's value or resolvedModel
 * wins first; failing that, a row whose value/resolvedModel extends the
 * requested id AT A COMPONENT BOUNDARY matches — the dated/context-suffix
 * tolerance that lets the bare "claude-opus-4-8" match the catalog's
 * "claude-opus-4-8[1m]" and "claude-haiku-4-5" match
 * "claude-haiku-4-5-20251001", without letting "c" match anything at all.
 */
export function matchModel(
  models: Array<{ value: string; resolvedModel?: string }>,
  requested: string,
): string | null {
  const exact = models.find((m) => m.value === requested || m.resolvedModel === requested);
  if (exact !== undefined) return exact.resolvedModel ?? requested;
  const prefixed = models.find(
    (m) =>
      idExtendsAtBoundary(m.value ?? "", requested) ||
      idExtendsAtBoundary(m.resolvedModel ?? "", requested),
  );
  if (prefixed !== undefined) return prefixed.resolvedModel ?? requested;
  return null;
}

/**
 * Build the async sdk_apply handler. Callers fire it without awaiting
 * (`void handler(params)`) so it never blocks the notification dispatcher;
 * every path settles by notifying a result (or logging a drop for a payload
 * the box could never have sent).
 */
export function createSdkApplyHandler(deps: SdkApplyDeps): (params: unknown) => Promise<void> {
  return async (params: unknown): Promise<void> => {
    const p = (params ?? {}) as { rid?: unknown; model?: unknown; effort?: unknown };
    const hasModel = typeof p.model === "string";
    const hasEffort = typeof p.effort === "string";
    if (typeof p.rid !== "string" || p.rid === "" || (!hasModel && !hasEffort)) {
      // No rid → no way to correlate an answer; no field → nothing to do. The
      // box's pending timer surfaces harness_unreachable. Never happens to the
      // shipped box.
      deps.log.warn("dropping malformed sdk_apply notification");
      return;
    }
    const rid = p.rid;
    const model = hasModel ? (p.model as string) : undefined;
    const effort = hasEffort ? (p.effort as string) : undefined;

    const session = deps.getSession();
    if (session === null) {
      deps.notifyResult({ rid, ok: false, code: "no_session", detail: "SDK session not running" });
      return;
    }
    const q = session.query;
    const generation = session.generation;
    /** Is the session we captured still the one that is running? Re-checked
     * after EVERY await: a stale true here is how an apply lands on a dead
     * query and is reported as success for a live one. */
    const stillLive = (): boolean => {
      const now = deps.getSession();
      return now !== null && now.generation === generation && now.query === q;
    };
    const abandon = (): void => {
      deps.log.warn(`sdk_apply ${rid}: session changed mid-apply; abandoning`);
      deps.notifyResult({
        rid,
        ok: false,
        code: "no_session",
        detail: "the SDK session restarted while applying; try again",
      });
    };

    // ---- Validate BEFORE touching the session (§5 of the design spec) --------
    // An unknown model / invalid effort is refused without ever issuing a
    // control. "" (clear) skips validation — reverting to the SDK default is
    // always legal.
    let resolvedModel: string | undefined;
    let modelUnverified = false;
    if (model !== undefined && model !== "") {
      let models: Array<{ value: string; resolvedModel?: string }>;
      try {
        models = await q.supportedModels();
      } catch (err) {
        deps.notifyResult({ rid, ok: false, code: "apply_failed", detail: firstLine((err as Error).message) });
        return;
      }
      // The longest await on this path, and the one the original race was found
      // on: query A can die and B start while the catalog is being fetched.
      if (!stillLive()) {
        abandon();
        return;
      }
      const matched = matchModel(models, model);
      if (matched === null) {
        // Tier 2: not in the catalog, but the box already gated it through the
        // syntactic regex, so it is a well-formed id. The catalog is not the
        // truth — the API is — and it under-enumerates live-valid ids (e.g.
        // claude-opus-4-6). Apply anyway, restamp the requested id, and carry
        // unverified so the client can caution: an API rejection would surface
        // on the next turn and is recoverable by flipping back.
        resolvedModel = model;
        modelUnverified = true;
      } else {
        resolvedModel = matched;
      }
    }

    let effortApply: ReturnType<typeof effortToSdkApply> | null = null;
    if (effort !== undefined) {
      effortApply = effortToSdkApply(effort);
      if (effortApply.kind === "invalid") {
        deps.notifyResult({ rid, ok: false, code: "invalid_effort", detail: `effort not recognized: ${firstLine(effort)}` });
        return;
      }
    }

    // ---- Apply to the LIVE session ------------------------------------------
    // Effort first (pre-validated, near-infallible), model second (the guarded
    // control). Each awaited so a CLI rejection surfaces as a thrown error.
    // `applied` accumulates what actually landed so a partial failure still
    // restamps the truth (persist_failed-style honesty).
    //
    // EFFORT IS REPLACEMENT STATE. The SDK has two independent effort controls
    // — the symbolic effortLevel flag and the raw maxThinkingTokens budget —
    // and effortToSdkApply picks exactly one per value. Setting only the chosen
    // one (as this used to) leaves the PREVIOUS mode's control still in force:
    // a numeric→symbolic change applied effortLevel on top of a live token
    // budget, and symbolic→numeric set a budget while the old effortLevel flag
    // stayed on. Live behaviour then diverged from a clean boot with the same
    // .env, which is the one invariant the shared effortToSdkApply mapping
    // exists to hold. So every transition clears the OPPOSITE control first.
    const applied: SdkApplied = {};
    try {
      if (effortApply !== null) {
        if (effortApply.kind === "effortLevel") {
          await q.setMaxThinkingTokens(null);
          await q.applyFlagSettings({ effortLevel: effortApply.level });
        } else if (effortApply.kind === "maxThinkingTokens") {
          await q.applyFlagSettings({ effortLevel: null });
          await q.setMaxThinkingTokens(effortApply.tokens);
        } else {
          // clear: drop both the effort-level flag and any thinking budget.
          await q.applyFlagSettings({ effortLevel: null });
          await q.setMaxThinkingTokens(null);
        }
        applied.effort = effort; // "" for clear
      }
      if (model !== undefined) {
        await q.setModel(model === "" ? undefined : model);
        applied.model = model; // "" for clear
        applied.resolvedModel = resolvedModel; // undefined for clear
      }
    } catch (err) {
      // Restamp whatever landed before the throw so harness_info stays honest —
      // but only if it landed on the session that is still running. Restamping
      // for a dead query would advertise an identity no live session has.
      if ((applied.model !== undefined || applied.effort !== undefined) && stillLive()) {
        deps.onApplied(applied);
      }
      deps.notifyResult({ rid, ok: false, code: "apply_failed", detail: firstLine((err as Error).message) });
      return;
    }

    // Last fence before the two irreversible steps. onApplied restamps
    // harness_info (the box's live identity, and therefore its no-op check) and
    // the ok result makes the box persist to .env. Doing either for a session
    // that died mid-apply is how a dead query's model becomes the box's
    // advertised truth.
    if (!stillLive()) {
      abandon();
      return;
    }

    // Mirror-then-announce ordering: local env mirror + harness_info restamp
    // first, the ok result second, so by the time the box persists and answers
    // the app its identity already reports the new values. One restamp, one
    // result — even for a combined model+effort apply.
    deps.onApplied(applied);
    const parts: string[] = [];
    if (applied.model !== undefined) parts.push(`model=${applied.model === "" ? "(default)" : applied.model}`);
    if (applied.effort !== undefined) parts.push(`effort=${applied.effort === "" ? "(default)" : applied.effort}`);
    deps.log.info(`sdk_apply ${rid}: ${parts.join(" ")} ok`);
    const result: SdkApplyResult = { rid, ok: true };
    if (applied.model !== undefined) result.model = applied.model;
    if (applied.effort !== undefined) result.effort = applied.effort;
    if (modelUnverified) result.unverified = true;
    deps.notifyResult(result);
  };
}
