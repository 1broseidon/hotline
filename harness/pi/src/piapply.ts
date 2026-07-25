/**
 * Pi hot apply (pi hot-apply amendment 2026-07-20) — the pi counterpart of
 * harness/claude-sdk/src/sdkapply.ts.
 *
 * The box forwards a hot-capable set_sdk_config as a
 * notifications/hotline/sdk_apply notification over the `hotline run` child's
 * stdio; this handler applies model and/or thinking level to the LIVE pi
 * session — no restart, the wire never drops — and answers on
 * notifications/hotline/sdk_apply_result.
 *
 * The topology is inverted from claude-sdk (pi hosts US; there we host the
 * SDK), but the notification contract is identical, so the box needed no new
 * frames — only a knob family.
 *
 * Two things pi gives us that the Agent SDK does not:
 *
 *   1. REAL validation. `resolveCliModel` resolves a pattern against the live
 *      ModelRegistry — the same resolver the `--provider/--model` CLI flags
 *      use, so a string that works on the launch line works here — and
 *      `ExtensionAPI.setModel` returns FALSE when the resolved model has no
 *      API key. Both are hard answers, not the SDK's fire-and-forget setModel,
 *      so there is no "unverified" tier here: a pi apply either resolved and
 *      authenticated, or it failed with a reason.
 *
 *   2. CLAMPING. `setThinkingLevel` silently clamps to the model's capability,
 *      so we read the level BACK after setting and report the clamped truth.
 *      The box persists and echoes that, which is what keeps the client's
 *      identity confirmation from hanging on a level the model can't do.
 *
 * Clear semantics ("" on the wire). claude-sdk can genuinely revert to an SDK
 * default; pi cannot — it always has exactly one live model and one live
 * thinking level. So a clear here means "stop PINNING this knob": nothing is
 * applied to the session, the box removes the .env line, and the live value
 * keeps being reported. The client's identityMatches accepts any identity for
 * a cleared field, so it confirms immediately. (The app's pi picker does not
 * offer the clear escape for exactly this reason; the wire still accepts it.)
 */

import type { Logger } from "@1broseidon/hotline-harness-core/log";

/**
 * Pi's ThinkingLevel literals (@earendil-works/pi-agent-core). Mirrored as a
 * plain union so this module stays testable without the pi package loaded.
 */
export type PiThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";

/**
 * The effort mapping table (pi hot-apply amendment 2026-07-20). Our wire effort
 * vocabulary is the claude-sdk ladder — low|medium|high|xhigh|max, plus "" =
 * clear — and every one of those five is a pi ThinkingLevel verbatim, so the
 * mapping is the identity.
 *
 * The two extra pi levels, "off" and "minimal", are ACCEPTED but not offered by
 * the app's picker. The reason is display fidelity: George can set either from
 * pi's own TUI, and the bidirectional restamp then reports it on the wire. If
 * the box refused those strings, a level pi is genuinely running could not
 * round-trip, and the app would have to either hide it or lie. Accepting the
 * full pi ladder inbound while curating the picker outbound keeps the chip
 * honest without widening what the app can DO to a box.
 *
 * Deliberately absent: the claude-sdk raw-integer form (a maxThinkingTokens
 * budget). Pi has no token-budget knob — setThinkingLevel takes a level — so a
 * digit string is invalid_effort here, and config.ValidPiThinking on the box
 * refuses it before it is ever forwarded.
 */
export const PI_THINKING_LEVELS: readonly PiThinkingLevel[] = [
  "off",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
];

/** Map a wire effort string onto a pi thinking level. "" = clear (apply
 * nothing live; the box just drops the persisted pin). */
export type PiEffortPlan =
  | { kind: "level"; level: PiThinkingLevel }
  | { kind: "clear" }
  | { kind: "invalid" };

export function planEffort(effort: string): PiEffortPlan {
  if (effort === "") return { kind: "clear" };
  const level = effort.trim().toLowerCase();
  return (PI_THINKING_LEVELS as readonly string[]).includes(level)
    ? { kind: "level", level: level as PiThinkingLevel }
    : { kind: "invalid" };
}

/** A model resolved against pi's live ModelRegistry. `canonical` is the
 * "provider/id" form we report and the box persists — the self-describing
 * shape `--model` re-resolves at the next respawn without a --provider flag. */
export interface PiResolvedModel {
  /** The opaque pi Model object handed straight back to ExtensionAPI.setModel. */
  model: unknown;
  provider: string;
  id: string;
  canonical: string;
  /** A ":level" suffix parsed out of the pattern ("glm-5.2:high"), if any. */
  thinkingLevel?: PiThinkingLevel;
}

/** The sdk_apply_result wire payload — the SAME shape the claude-sdk harness
 * answers with (internal box↔harness stdio contract, not the app wire). */
export interface PiApplyResult {
  rid: string;
  ok: boolean;
  /** On ok: the CANONICAL applied model, present iff the request carried model
   * ("" for clear). Canonical, not requested — the box persists this. */
  model?: string;
  /** On ok: the applied thinking level AFTER pi's clamp, present iff the
   * request carried effort ("" for clear). */
  effort?: string;
  code?: PiApplyFailureCode;
  /** Single line, ≤ 200 chars. */
  detail?: string;
}

/**
 * The closed failure set. `unknown_model` and `apply_failed` are shared with
 * the claude-sdk path (same client copy); `no_api_key` is pi-specific and
 * earns its own code because it is the one failure the operator can FIX (log
 * into the provider) rather than retry — collapsing it into unknown_model
 * would send them hunting for a typo in a model id that resolved perfectly.
 */
export type PiApplyFailureCode =
  | "unknown_model"
  | "no_api_key"
  | "invalid_effort"
  | "apply_failed"
  | "no_session";

/** What onApplied conveys: only the fields the request actually changed. */
export interface PiApplied {
  /** present iff the request carried model ("" = unpinned). */
  model?: string;
  /** present iff the request carried effort ("" = unpinned). */
  effort?: string;
}

export interface PiApplyDeps {
  /**
   * Resolve a model pattern against pi's live registry. Returns null when
   * nothing matches (→ unknown_model). Never throws for a miss; a THROW here
   * is a registry failure and surfaces as apply_failed.
   */
  resolveModel: (pattern: string) => PiResolvedModel | null;
  /** ExtensionAPI.setModel — false means "no API key for that model". */
  setModel: (model: unknown) => Promise<boolean>;
  /** ExtensionAPI.getThinkingLevel — read back AFTER a set to see the clamp. */
  getThinkingLevel: () => PiThinkingLevel;
  /** ExtensionAPI.setThinkingLevel — clamps to the model's capability. */
  setThinkingLevel: (level: PiThinkingLevel) => void;
  /**
   * The live session's generation token, or null before session_start / after
   * session_shutdown (→ no_session).
   *
   * A number rather than a boolean because an apply is several awaits long and
   * `isReady()` taken once at entry cannot see the session being replaced
   * inside them: pi can shut a session down while setModel is in flight, and
   * the handler would then set a thinking level and restamp harness_info
   * against a session that is already gone — advertising an identity nothing
   * is running. Re-reading the token after every await is what catches that.
   */
  getSession: () => number | null;
  /** Sends an sdk_apply_result notification back to the run child. */
  notifyResult: (result: PiApplyResult) => void;
  /**
   * Called after a successful apply and BEFORE the ok result is notified: the
   * extension updates its HOTLINE_PI_* env mirrors and restamps harness_info,
   * so the box's live identity — and therefore its no-op check — tells the
   * truth before the box answers the app. On a partial failure (model applied,
   * effort threw) this still fires for the part that landed.
   */
  onApplied: (applied: PiApplied) => void;
  log: Logger;
}

/** The §5.3 harness_info payload the extension announces to the run child.
 *
 * PRESENCE, not just value (hot-clear amendment): an omitted field means "not
 * reported, leave the box's value alone"; "" means "reported as unpinned". Pi
 * always has a live model and a live thinking level, so in practice these carry
 * values — but an unpin has to be SAYABLE, or the box keeps advertising a pin
 * that no longer exists and its no-op check keeps believing it. */
export interface PiHarnessInfo {
  harness: "pi";
  model?: string;
  effort?: string;
}

/**
 * The restamp-storm guard for bidirectional sync.
 *
 * pi fires model_select / thinking_level_select for OUR OWN setModel and
 * setThinkingLevel calls, not just for the operator's TUI changes, so a single
 * combined hot apply would otherwise announce three times (model_select,
 * thinking_level_select, and the explicit post-apply restamp) — and pi's model
 * cycle can re-select the same model repeatedly. `announce` therefore emits
 * only when the announced identity actually CHANGED.
 *
 * `send` returns false when the child is down; a dropped announcement is not
 * recorded, so the next child-ready resends it. `force` bypasses the guard for
 * exactly that case: a freshly-ready child holds only the box's config seed and
 * must be told the live values even if they match the last announcement.
 */
export class IdentityAnnouncer {
  private last: string | null = null;

  constructor(private readonly send: (info: PiHarnessInfo) => boolean) {}

  /** Forget the last announcement (session boundaries). */
  reset(): void {
    this.last = null;
  }

  /** Returns true iff a notification was actually sent. */
  announce(model: string | undefined, effort: string | undefined, force = false): boolean {
    // undefined = not known, so omit. "" = explicitly unpinned, so SEND it —
    // dropping it (the old `if (model)` truthiness test did) is what left the
    // box advertising a cleared pin.
    const key = `${model ?? "\u0000"}|${effort ?? "\u0000"}`;
    if (!force && key === this.last) return false;
    const info: PiHarnessInfo = { harness: "pi" };
    if (model !== undefined) info.model = model;
    if (effort !== undefined) info.effort = effort;
    if (!this.send(info)) return false;
    this.last = key;
    return true;
  }
}

/** First line of a message, capped at 200 chars (the result detail cap). */
export function firstLine(s: string): string {
  const line = s.split("\n", 1)[0] ?? "";
  return line.length > 200 ? line.slice(0, 200) : line;
}

/**
 * Build the async sdk_apply handler. Callers fire it without awaiting
 * (`void handler(params)`) so it never blocks the notification dispatcher;
 * every path settles by notifying a result (or logging a drop for a payload
 * the box could never have sent).
 */
export function createPiApplyHandler(deps: PiApplyDeps): (params: unknown) => Promise<void> {
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

    const generation = deps.getSession();
    if (generation === null) {
      deps.notifyResult({ rid, ok: false, code: "no_session", detail: "pi session not running" });
      return;
    }
    /** Is the session we captured still the one that is running? */
    const stillLive = (): boolean => deps.getSession() === generation;
    const abandon = (): void => {
      deps.log.warn(`sdk_apply ${rid}: session changed mid-apply; abandoning`);
      deps.notifyResult({
        rid,
        ok: false,
        code: "no_session",
        detail: "the pi session restarted while applying; try again",
      });
    };

    // ---- Validate BEFORE touching the session -------------------------------
    // Resolution and effort parsing both happen up front, so an unknown model
    // or a bogus level is refused without half-applying the other knob.
    let resolved: PiResolvedModel | null = null;
    if (model !== undefined && model !== "") {
      try {
        resolved = deps.resolveModel(model);
      } catch (err) {
        deps.notifyResult({ rid, ok: false, code: "apply_failed", detail: firstLine((err as Error).message) });
        return;
      }
      if (resolved === null) {
        deps.notifyResult({
          rid,
          ok: false,
          code: "unknown_model",
          detail: `no model matches ${firstLine(model)}`,
        });
        return;
      }
    }

    let effortPlan: PiEffortPlan | null = null;
    if (effort !== undefined) {
      effortPlan = planEffort(effort);
      if (effortPlan.kind === "invalid") {
        deps.notifyResult({ rid, ok: false, code: "invalid_effort", detail: `effort not recognized: ${firstLine(effort)}` });
        return;
      }
    }

    // ---- Apply to the LIVE session ------------------------------------------
    // MODEL FIRST, then effort — the opposite order from claude-sdk, and
    // deliberately so: setThinkingLevel clamps against the CURRENT model's
    // capability, so the model has to land before the level is chosen or a
    // level the new model supports could be clamped against the old one.
    // `applied` accumulates what actually landed so a partial failure still
    // restamps the truth.
    const applied: PiApplied = {};
    try {
      if (resolved !== null) {
        const ok = await deps.setModel(resolved.model);
        if (!ok) {
          // setModel's documented false: the model resolved fine, there is just
          // no credential for its provider on this box. Nothing changed.
          deps.notifyResult({
            rid,
            ok: false,
            code: "no_api_key",
            detail: `no API key configured for ${firstLine(resolved.provider)}`,
          });
          return;
        }
        // setModel is the one real await here, and the window the fence exists
        // for: a session_shutdown inside it must not be followed by a thinking
        // level set against the dead session.
        if (!stillLive()) {
          abandon();
          return;
        }
        applied.model = resolved.canonical;
      } else if (model === "") {
        // Unpin: no live change, the box just drops the persisted line.
        applied.model = "";
      }

      if (effortPlan !== null && effortPlan.kind === "level") {
        deps.setThinkingLevel(effortPlan.level);
        // Read back: pi clamps to the model's capability, and the clamped value
        // is the one the box must persist and the client must confirm against.
        applied.effort = deps.getThinkingLevel();
      } else if (effortPlan !== null) {
        applied.effort = ""; // unpin
      } else if (resolved !== null) {
        // A ":level" suffix inside the model pattern ("glm-5.2:high") is a
        // thinking selection the operator wrote into the model string. Honor it
        // only when the request carried NO explicit effort, so an explicit
        // effort field always wins.
        if (resolved.thinkingLevel !== undefined) {
          deps.setThinkingLevel(resolved.thinkingLevel);
        }
      }
    } catch (err) {
      // Restamp whatever landed before the throw so harness_info stays honest —
      // but only for a session that is still running.
      if ((applied.model !== undefined || applied.effort !== undefined) && stillLive()) {
        deps.onApplied(applied);
      }
      deps.notifyResult({ rid, ok: false, code: "apply_failed", detail: firstLine((err as Error).message) });
      return;
    }

    // Last fence before the two irreversible steps: onApplied restamps the
    // box's live identity, and the ok result makes the box persist to .env.
    if (!stillLive()) {
      abandon();
      return;
    }

    // Mirror-then-announce ordering: local env mirror + harness_info restamp
    // first, the ok result second, so by the time the box persists and answers
    // the app its identity already reports the new values.
    deps.onApplied(applied);
    const parts: string[] = [];
    if (applied.model !== undefined) parts.push(`model=${applied.model === "" ? "(unpinned)" : applied.model}`);
    if (applied.effort !== undefined) parts.push(`effort=${applied.effort === "" ? "(unpinned)" : applied.effort}`);
    deps.log.info(`sdk_apply ${rid}: ${parts.join(" ")} ok`);
    const result: PiApplyResult = { rid, ok: true };
    if (applied.model !== undefined) result.model = applied.model;
    if (applied.effort !== undefined) result.effort = applied.effort;
    deps.notifyResult(result);
  };
}
