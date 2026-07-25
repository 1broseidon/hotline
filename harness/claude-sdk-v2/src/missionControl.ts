/**
 * Mission Control handoff loop for the claude-sdk harness (spec workstream 2).
 *
 * This is a *reduced* re-implementation of pi's `harness/pi/src/missionControl.ts`
 * decision core — NOT a port of `MissionControlLoop`. The SDK's seams differ
 * enough that a shared-core extraction would force a pi edit and a new core
 * parity row for four constant strings, so the pure prompt/instruction/threshold
 * constants below are duplicated from pi *byte-for-byte* on purpose. A core
 * `mission.ts` extraction is a sanctioned follow-up if a third harness ever
 * grows a loop.
 *
 * What the SDK gives us (verified against @anthropic-ai/claude-agent-sdk@0.3.215):
 *   - `Query.getContextUsage()` per settle (equal to pi's ctx.getContextUsage).
 *   - a `UserPromptSubmit` hook whose `additionalContext` injects the 80% nudge
 *     once (equal in effect to pi's before_agent_start system-prompt append).
 *   - `/compact <instructions>` as a streaming user turn — instructions DO reach
 *     the summarizer (better than pi 0.80.6). There is NO `compact()` control.
 *   - a `PreCompact` hook that is AWAITED before compaction proceeds but CANNOT
 *     cancel it.
 *
 * Because PreCompact cannot cancel, the loop splits by who initiates compaction:
 *   - Cap-initiated (we cross HOTLINE_MC_CONTEXT_CAP): full pi fidelity — queue a
 *     real handoff turn first, then the mechanical fallback if the model wrote
 *     nothing, then `/compact <COMPACTION_INSTRUCTIONS>`.
 *   - CLI-auto-initiated (CC's autoCompact fires first): the awaited PreCompact
 *     hook runs the mechanical `hotline mission handoff --trigger auto` when no
 *     fresh handoff exists — compaction proceeds only after it lands on disk.
 *     Stronger than pi's layer 2, weaker than pi's layer 3 (no model turn).
 *
 * Reductions vs pi's class: no recompacting/cancel bookkeeping (nothing to
 * cancel) and no settle-generation fencing (the AsyncQueue serializes our own
 * injected turns, so the next `result` after a queued handoff turn is that
 * turn's settle). The cap cycle is a plain state machine:
 *   idle → handoffPending → compactQueued → (reset on compact_boundary) → idle.
 * A queued `/compact` that the CLI short-circuits (nothing to compact) emits an
 * ordinary result with NO compact_boundary; the compactQueued settle detects the
 * missing boundary and resets to idle so the loop cannot wedge (see onResult).
 *
 * The decision core here is pure and unit-tested (test/mission.test.mjs); the
 * hook/queue wiring in index.ts is defensively caught everywhere (this runs
 * inside the SDK session loop and must never throw a session down).
 */

/** The context-usage shape the loop measures against. Mapped from the SDK's
 * getContextUsage() response by usageFromSdk(). */
export interface ContextUsageLike {
  tokens: number | null;
  contextWindow: number;
}

/**
 * Map the SDK's `getContextUsage()` response
 * (`{ totalTokens, maxTokens, … }`) onto the loop's ContextUsageLike. A missing
 * numeric total reads as null (treated as "unknown", resets the nudge gate).
 */
export function usageFromSdk(res: { totalTokens?: unknown; maxTokens?: unknown }): ContextUsageLike {
  return {
    tokens: typeof res.totalTokens === "number" ? res.totalTokens : null,
    contextWindow: typeof res.maxTokens === "number" ? res.maxTokens : 0,
  };
}

/** Parse HOTLINE_MC_CONTEXT_CAP into a positive token count, or null when
 * unset/invalid. Copied byte-for-byte from pi missionControl.ts:75-81. */
export function parseContextCap(env: NodeJS.ProcessEnv = process.env): number | null {
  const raw = (env.HOTLINE_MC_CONTEXT_CAP ?? "").trim();
  if (raw === "") return null;
  const n = Number(raw);
  if (!Number.isFinite(n) || n <= 0) return null;
  return Math.floor(n);
}

/**
 * The effective cap the loop measures against: the configured soft cap when set,
 * otherwise the SDK's own context window (maxTokens). Returns null when neither
 * is usable. (pi clamps against keepRecentTokens; CC has no retained-window
 * setting the harness must respect, so the clamp collapses to a maxTokens
 * sanity — spec §2 SDK constraint map.)
 */
export function effectiveCap(usage: ContextUsageLike, cap: number | null): number | null {
  if (cap && cap > 0) return cap;
  if (usage.contextWindow && usage.contextWindow > 0) return usage.contextWindow;
  return null;
}

/** Fraction of the effective cap currently used, or null when it can't be
 * computed. Copied from pi missionControl.ts:108-113. */
export function usageFraction(usage: ContextUsageLike, cap: number | null): number | null {
  if (usage.tokens == null) return null;
  const eff = effectiveCap(usage, cap);
  if (eff == null) return null;
  return usage.tokens / eff;
}

/** The system-prompt nudge line, parameterized by the current usage percent.
 * Copied byte-for-byte from pi missionControl.ts:116-118. */
export function nudgeLine(percent: number): string {
  return `Context is at ${percent}% of its working budget — write a mission handoff (action: handoff, trigger: pre-compact) in your next reply so the next session resumes cleanly, then continue.`;
}

/** The `/compact` summary insurance instructions. Byte-for-byte from pi
 * missionControl.ts:121-122. Unlike pi 0.80.6 these DO reach the summarizer. */
export const COMPACTION_INSTRUCTIONS =
  "Append exactly this final block and fill every field:\nMISSION HANDOFF\nDoing: ...\nState: ...\nNext: ...\nBeware: ...";

/** The user-turn prompt that asks the agent to write its handoff before
 * compaction. Byte-for-byte from pi missionControl.ts:125-126. */
export const HANDOFF_TURN_PROMPT =
  "[mission-control] Your context is about to be compacted. Before it is, write a mission handoff now: call the mission tool with action \"handoff\", trigger \"pre-compact\", a `state` describing what you're doing and where it stands, and a `next` for the first thing to do after. Keep working right after.";

/** Threshold constants (spec §4). Byte-for-byte from pi missionControl.ts:129-139. */
export const NUDGE_AT = 0.8;
export const NUDGE_STEP = 0.1;
export const HANDOFF_FRESH_WINDOW = 0.25;

/** The mechanical fallback's fixed `--next`. */
const MECHANICAL_NEXT = "re-read the thread you were on and continue";

/** Actions the loop performs on the live SDK session. Injected for testability. */
export interface SdkLoopActions {
  /** Arm the 80% nudge for the next UserPromptSubmit hook. */
  armNudge(line: string): void;
  /** Queue the handoff-turn prompt as a real user turn (queue.push). */
  sendHandoffTurn(prompt: string): void;
  /** Queue `/compact <instructions>` as a streaming user turn (queue.push). */
  queueCompact(instructions: string): void;
  /** Run the mechanical fallback: `hotline mission handoff --trigger auto …`.
   * Returns a promise on the PreCompact path (awaited so compaction proceeds
   * only after it lands); ignored on the cap path (fire-and-forget). */
  mechanicalHandoff(state: string, next: string): void | Promise<void>;
  log(msg: string): void;
  warn(msg: string): void;
}

type CapState = "idle" | "handoffPending" | "compactQueued";

/**
 * SdkMissionLoop is the reduced pure decision core. Every side effect goes
 * through SdkLoopActions so the whole state machine is unit-testable with no
 * SDK, queue, or child process in sight.
 */
export class SdkMissionLoop {
  private state: CapState = "idle";
  private lastNudgeFraction: number | null = null;
  // True once the model actually wrote a handoff via the mission tool during the
  // current pending turn (observed by the PostToolUse hook). Reset when a fresh
  // handoff turn begins and on compaction.
  private handoffObserved = false;
  // Token count at which the most recent handoff was written, for the freshness
  // window. null ⇒ no handoff written this cycle.
  private handoffTokens: number | null = null;
  private lastTokens: number | null = null;
  private lastEffCap: number | null = null;
  private lastUserMsg = "";

  constructor(
    private readonly cap: number | null,
    private readonly actions: SdkLoopActions,
  ) {}

  /** Whether a soft cap is configured. */
  hasCap(): boolean {
    return this.cap != null;
  }

  /** Record the latest real user prompt (for the mechanical fallback's state
   * note). index.ts filters out our own injected turns before calling this. */
  noteUserPrompt(prompt: string): void {
    this.lastUserMsg = prompt;
  }

  /**
   * Called once per `result` message with the latest context usage. Arms the
   * 80% nudge, advances a pending handoff turn (this result is its settle), and,
   * when a soft cap is set, drives the cap cycle once usage crosses it.
   */
  onResult(usage: ContextUsageLike): void {
    this.lastTokens = usage.tokens;
    this.lastEffCap = effectiveCap(usage, this.cap);
    const frac = usageFraction(usage, this.cap);
    if (frac == null) {
      // tokens null (e.g. right after compaction) resets the nudge gate.
      this.lastNudgeFraction = null;
      return;
    }

    // Layer 1: arm the nudge at 80%, re-arm every +10% growth.
    if (
      frac >= NUDGE_AT &&
      (this.lastNudgeFraction == null || frac - this.lastNudgeFraction >= NUDGE_STEP)
    ) {
      this.lastNudgeFraction = frac;
      this.actions.armNudge(nudgeLine(Math.round(frac * 100)));
    }

    // The cap cycle's second beat: the handoff turn we queued has now settled
    // (its `result` is this call). Fire the mechanical fallback if the model
    // wrote nothing, then queue the real compaction.
    if (this.state === "handoffPending") {
      this.state = "compactQueued";
      if (!this.handoffObserved) {
        this.actions.log("handoff turn wrote no handoff; using the mechanical fallback");
        this.actions.mechanicalHandoff(this.fallbackState(), MECHANICAL_NEXT);
      }
      this.actions.queueCompact(COMPACTION_INSTRUCTIONS);
      return;
    }

    // A compaction is queued. The queue serializes our own injected turns, so
    // the first `result` after queueCompact is that `/compact` turn's own settle
    // (spec §2 reduction). On a REAL compaction the `compact_boundary` system
    // message fires first and onCompactBoundary has already returned us to idle
    // — so reaching onResult while STILL compactQueued means the CLI
    // short-circuited the `/compact` (the observed bailout is an ordinary
    // assistant+result turn, e.g. "Not enough messages to compact.", with NO
    // boundary ever emitted). That used to wedge the loop forever: compact_boundary
    // is the only other reset, so nudge + cap enforcement went silent until a
    // respawn. Treat the boundary-less settle as a benign no-op and return to
    // idle so the cap layer can re-arm on the next threshold crossing — pi's
    // benign-compact-rejection semantics (missionControl.ts:378-391, which also
    // only release the cycle latch and keep handoff freshness). Structural, not
    // string-matched: any queued-compact settle without a boundary resets, so a
    // future bailout-text change cannot re-wedge it. Do NOT re-enter cap logic
    // this call; the next real settle re-evaluates the cap (pi returns from
    // onCompactRejected the same way).
    if (this.state === "compactQueued") {
      this.actions.log("queued /compact produced no compaction boundary (no-op); resetting cycle to idle");
      this.state = "idle";
      return;
    }

    // Cap enforcement (idle). Crossing the cap with a fresh handoff on disk
    // compacts straight away; otherwise run a real handoff turn first (P2-B).
    if (this.cap != null && usage.tokens != null && usage.tokens >= this.cap) {
      if (this.handoffIsFresh()) {
        this.state = "compactQueued";
        this.actions.log(`context ${usage.tokens} >= cap ${this.cap}; fresh handoff on disk, compacting`);
        this.actions.queueCompact(COMPACTION_INSTRUCTIONS);
      } else {
        this.state = "handoffPending";
        this.handoffObserved = false;
        this.actions.log(`context ${usage.tokens} >= cap ${this.cap}; running a handoff turn before compaction`);
        this.actions.sendHandoffTurn(HANDOFF_TURN_PROMPT);
      }
    }
  }

  /**
   * The CLI-auto divergence (spec §2 resolution). Called from the awaited
   * PreCompact hook. A user `/compact` (trigger "manual", including our own
   * cap-path `/compact`) is a now request and is left alone. An automatic
   * compaction with no fresh handoff returns the mechanical fallback the hook
   * must run (and await) before compaction proceeds.
   */
  onPreCompact(trigger: "manual" | "auto"): { mechanical: { state: string; next: string } | null } {
    if (trigger === "manual") return { mechanical: null };
    if (this.handoffIsFresh()) return { mechanical: null };
    // Stamp so a second PreCompact within the same boundary does not double-fire
    // (the mechanical handoff we are about to write counts as a fresh handoff).
    this.handoffTokens = this.lastTokens;
    return { mechanical: { state: this.fallbackState(), next: MECHANICAL_NEXT } };
  }

  /** Record that the agent called the mission handoff tool (PostToolUse hook,
   * matcher mcp__hotline__mission, action "handoff", non-error result). Stamps
   * the token count for the freshness window. */
  noteHandoffWritten(): void {
    this.handoffObserved = true;
    this.handoffTokens = this.lastTokens;
  }

  /** Reset for the next cycle, driven by the compact_boundary stream message
   * (and idempotent if a PostCompact hook also fires). */
  onCompactBoundary(): void {
    this.state = "idle";
    this.handoffObserved = false;
    this.handoffTokens = null;
    this.lastNudgeFraction = null;
    this.lastTokens = null;
    this.lastEffCap = null;
  }

  /**
   * A handoff only suppresses a pre-compaction turn if it was written recently —
   * inside the last HANDOFF_FRESH_WINDOW of the effective cap. Unknown token
   * info can't be judged, so it counts as fresh. Copied from pi
   * missionControl.ts:285-289.
   */
  private handoffIsFresh(): boolean {
    if (this.handoffTokens == null) return false;
    if (this.lastEffCap == null || this.lastEffCap <= 0) return true;
    return this.handoffTokens >= (1 - HANDOFF_FRESH_WINDOW) * this.lastEffCap;
  }

  private fallbackState(): string {
    const tokens = this.lastTokens != null ? String(this.lastTokens) : "unknown";
    return `compaction at ${tokens} tokens; last user message: ${truncate(this.lastUserMsg, 200)}`;
  }
}

function truncate(s: string, n: number): string {
  if (s.length <= n) return s;
  return s.slice(0, n);
}
