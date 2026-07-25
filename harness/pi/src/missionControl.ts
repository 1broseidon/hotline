/**
 * Mission Control handoff loop for the Pi harness (spec §4, §5).
 *
 * Three layers, best-effort first, mechanical last, plus the soft context cap:
 *
 *  1. Nudge (the good path): each turn we read pi's context estimate. Crossing
 *     80% of the effective cap arms a one-line system-prompt nudge, injected on
 *     the next before_agent_start, asking the model to write a mission handoff
 *     while it still holds full context. Re-armed at most once per 10% growth.
 *  2. Compaction-summary insurance where pi supports it: hotline-triggered
 *     compact() calls receive a short, imperative MISSION HANDOFF instruction
 *     directly. Pi 0.80.6 does not propagate session_before_compact instruction
 *     mutations to its summarizer, so pi-owned compactions rely on the durable
 *     handoff written by layers 1/3 instead of pretending the hook can alter it.
 *  3. Real handoff turn (resolved call #2): when compaction is about to run and
 *     no handoff was written recently, we cancel it, hand the agent one real
 *     turn to write its handoff through the mission tool, then re-trigger
 *     compaction. If that turn produces nothing, the mechanical fallback writes
 *     a thin `hotline mission handoff --trigger auto` so the next session still
 *     wakes holding *something*.
 *
 * HOTLINE_MC_CONTEXT_CAP (§5) is the soft cap: hotline cannot shrink a harness
 * window, only ask for compaction earlier. When the estimate crosses the cap we
 * call compact() (after the 80% nudge, which is computed against the same cap).
 * Unset ⇒ the harness's own threshold governs and we only ride
 * session_before_compact.
 *
 * The decision core here is pure and unit-tested (run-unit.mjs Part 7); index.ts
 * wires it to the real pi events with defensive try/catch everywhere (this code
 * runs inside pi's event loop and must never throw a session down).
 */

/** Minimal callback-based shape of pi's public ExtensionContext.compact(). */
export interface CompactContextLike {
  compact(options: {
    customInstructions?: string;
    onComplete?: () => void;
    onError?: (error: Error) => void;
  }): void;
}

/**
 * Convert pi's fire-and-forget compact callback API into a promise. The loop can
 * then attach one rejection handler that logs and releases all state latches.
 */
export function compactWithCallbacks(
  ctx: CompactContextLike,
  customInstructions: string,
): Promise<void> {
  return new Promise((resolve, reject) => {
    ctx.compact({
      customInstructions,
      onComplete: () => resolve(),
      onError: (error) => reject(error),
    });
  });
}

/** The context-usage shape pi exposes via ctx.getContextUsage(). */
export interface ContextUsageLike {
  tokens: number | null;
  contextWindow: number;
  percent: number | null;
}

/**
 * Safety room above pi's retained window. Four thousand tokens is large enough
 * to put a normal turn outside the 20k keep window, while keeping a mistaken low
 * cap from being inflated excessively. Turn boundaries still mean no token-only
 * margin can guarantee that every individual session is immediately compactable.
 */
export const COMPACTION_CAP_MARGIN = 4096;

/** Parse HOTLINE_MC_CONTEXT_CAP into a positive token count, or null when unset/invalid. */
export function parseContextCap(env: NodeJS.ProcessEnv = process.env): number | null {
  const raw = (env.HOTLINE_MC_CONTEXT_CAP ?? "").trim();
  if (raw === "") return null;
  const n = Number(raw);
  if (!Number.isFinite(n) || n <= 0) return null;
  return Math.floor(n);
}

/** Raise a configured cap above pi's retained compaction window. */
export function clampContextCap(
  cap: number | null,
  keepRecentTokens: number | null,
  margin = COMPACTION_CAP_MARGIN,
): number | null {
  if (cap == null) return null;
  if (keepRecentTokens == null || !Number.isFinite(keepRecentTokens) || keepRecentTokens < 0) {
    return cap;
  }
  const safeMargin = Number.isFinite(margin) && margin > 0 ? Math.floor(margin) : 0;
  return Math.max(cap, Math.floor(keepRecentTokens) + safeMargin);
}

/**
 * The effective cap the loop measures against: the configured soft cap when set,
 * otherwise pi's own context window. Returns null when neither is usable.
 */
export function effectiveCap(usage: ContextUsageLike, cap: number | null): number | null {
  if (cap && cap > 0) return cap;
  if (usage.contextWindow && usage.contextWindow > 0) return usage.contextWindow;
  return null;
}

/** Fraction of the effective cap currently used, or null when it can't be computed. */
export function usageFraction(usage: ContextUsageLike, cap: number | null): number | null {
  if (usage.tokens == null) return null;
  const eff = effectiveCap(usage, cap);
  if (eff == null) return null;
  return usage.tokens / eff;
}

/** The system-prompt nudge line, parameterized by the current usage percent. */
export function nudgeLine(percent: number): string {
  return `Context is at ${percent}% of its working budget — write a mission handoff (action: handoff, trigger: pre-compact) in your next reply so the next session resumes cleanly, then continue.`;
}

/** The direct compact() summary insurance; unsupported on before_compact. */
export const COMPACTION_INSTRUCTIONS =
  "Append exactly this final block and fill every field:\nMISSION HANDOFF\nDoing: ...\nState: ...\nNext: ...\nBeware: ...";

/** The user-turn prompt that asks the agent to write its handoff before compaction. */
export const HANDOFF_TURN_PROMPT =
  "[mission-control] Your context is about to be compacted. Before it is, write a mission handoff now: call the mission tool with action \"handoff\", trigger \"pre-compact\", a `state` describing what you're doing and where it stands, and a `next` for the first thing to do after. Keep working right after.";

/** Threshold constants (spec §4). */
export const NUDGE_AT = 0.8;
export const NUDGE_STEP = 0.1;

/**
 * P3 handoff-freshness window: a handoff only suppresses the pre-compaction
 * intercept if it was written while usage was inside the last 25% of the
 * effective cap/window (i.e. tokens-at-handoff ≥ 75% of the cap). A handoff
 * written earlier is treated as stale — the intercept still runs so the summary
 * that lands in the compacted window reflects current work.
 */
export const HANDOFF_FRESH_WINDOW = 0.25;

/** Actions the loop performs on the live pi session. Injected for testability. */
export interface LoopActions {
  /** Arm a system-prompt nudge for the next before_agent_start. */
  armNudge(line: string): void;
  /** Trigger pi compaction (optionally with extended custom instructions). */
  compact(customInstructions: string): void | Promise<void>;
  /** Send the handoff-turn prompt to the agent as a real user turn. */
  sendHandoffTurn(prompt: string): void;
  /** Run the mechanical fallback: `hotline mission handoff --trigger auto …`. */
  mechanicalHandoff(state: string, next: string): void;
  log(msg: string): void;
  warn(msg: string): void;
}

/**
 * MissionControlLoop holds the small amount of per-session state the layers need
 * and turns pi events into LoopActions calls. It is deliberately synchronous and
 * side-effect-only-through-actions so the whole state machine is unit-testable.
 */
export class MissionControlLoop {
  private lastNudgeFraction: number | null = null;
  private capCompactRequested = false;
  // Settle generation at which the real handoff turn was queued. The same
  // agent_settled handler that arms it must not consume it: only a strictly later
  // generation can represent the queued follow-up turn's own settlement.
  private handoffTurnArmedAtSettle: number | null = null;
  // Monotonic session-local generation. index.ts allocates one per
  // agent_settled event and passes that same number to both callbacks.
  private settleSequence = 0;
  // True between our re-triggered compact() and the before_compact it causes, so
  // that second before_compact proceeds instead of starting another handoff turn.
  // onCompacted resets success; requestCompact resets a rejection/no-op.
  private recompacting = false;
  // True once the agent has written (or been asked to write) a handoff in the
  // current pre-compaction cycle, so we don't loop.
  private handoffWritten = false;
  // Token count observed when the current handoff was written/armed, for the P3
  // freshness check. null ⇒ unknown (treat any handoff as fresh — can't judge).
  private handoffTokens: number | null = null;
  // Latest observed context token count, for the mechanical fallback's state note
  // and as the handoff timestamp source.
  private lastTokens: number | null = null;
  // Latest effective cap (soft cap or window), for the P3 freshness comparison.
  private lastEffCap: number | null = null;

  constructor(
    private readonly cap: number | null,
    private readonly actions: LoopActions,
  ) {}

  /** Whether a soft cap is configured. */
  hasCap(): boolean {
    return this.cap != null;
  }

  /** Allocate the generation for one agent_settled event. */
  beginAgentSettle(): number {
    this.settleSequence += 1;
    return this.settleSequence;
  }

  /**
   * Called each turn with the latest context usage. Arms the 80% nudge and, when
   * a soft cap is set, requests compaction once the estimate crosses it.
   * `settleSeq` must be shared with onHandoffTurnSettled when both run in the
   * same agent_settled handler. Omission allocates a generation for pure tests.
   */
  onContextUsage(usage: ContextUsageLike, settleSeq?: number): void {
    this.observeSettleSequence(settleSeq);
    this.lastTokens = usage.tokens;
    this.lastEffCap = effectiveCap(usage, this.cap);
    const frac = usageFraction(usage, this.cap);
    if (frac == null) {
      // tokens null (e.g. right after compaction) resets the nudge + cap gates.
      this.lastNudgeFraction = null;
      this.capCompactRequested = false;
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

    // §5: soft cap enforcement. P2-B: pi hard-codes reason "manual" in
    // session_before_compact for our own compact(), so before_compact can't run
    // the layer-3 handoff turn for the cap path. Drive it here instead: run the
    // handoff turn FIRST, then compact (recompacting armed) once it settles.
    if (this.cap != null && usage.tokens != null && usage.tokens >= this.cap) {
      // An automatic before_compact may have queued this handoff below the cap,
      // with its follow-up settling above it. That later settle is already going
      // to recompact in onHandoffTurnSettled; cap enforcement must not race it
      // with a second compact() request in the same generation.
      if (this.handoffTurnArmedAtSettle != null) {
        this.capCompactRequested = true;
        return;
      }
      if (!this.capCompactRequested) {
        this.capCompactRequested = true;
        this.actions.log(`context ${usage.tokens} >= cap ${this.cap}; requesting compaction`);
        this.startCapHandoff();
      }
    } else {
      this.capCompactRequested = false;
    }
  }

  /**
   * The cap path's entry into layer 3 (P2-B). A fresh handoff on disk means we
   * compact straight away (recompacting armed so the "manual" before_compact for
   * our own compact() passes through); otherwise we run a real handoff turn first
   * and let onHandoffTurnSettled fire the mechanical fallback + recompact.
   */
  private startCapHandoff(): void {
    if (this.handoffIsFresh()) {
      this.recompacting = true;
      this.requestCompact(COMPACTION_INSTRUCTIONS);
      return;
    }
    this.beginHandoffTurn("cap crossed with no fresh handoff; running a handoff turn before compaction");
  }

  /**
   * Arm and send a real handoff turn: cancel-then-recompact machinery shared by
   * the cap path (P2-B) and the before_compact intercept (layer 3). Records the
   * token count for the P3 freshness window.
   */
  private beginHandoffTurn(logMsg: string): void {
    this.handoffTurnArmedAtSettle = this.settleSequence;
    this.handoffWritten = true;
    this.handoffTokens = this.lastTokens;
    this.actions.log(logMsg);
    this.actions.sendHandoffTurn(HANDOFF_TURN_PROMPT);
  }

  /**
   * P3: a handoff only suppresses the intercept if it was written recently —
   * inside the last HANDOFF_FRESH_WINDOW of the effective cap. Unknown token
   * info can't be judged, so it counts as fresh (preserves prior behavior).
   */
  private handoffIsFresh(): boolean {
    if (!this.handoffWritten) return false;
    if (this.handoffTokens == null || this.lastEffCap == null || this.lastEffCap <= 0) return true;
    return this.handoffTokens >= (1 - HANDOFF_FRESH_WINDOW) * this.lastEffCap;
  }

  /**
   * Called on session_before_compact. `reason` is pi's compaction trigger
   * ("manual" | "threshold" | "overflow"). Returns whether to cancel this
   * compaction: true means we cancelled to run a real handoff turn (index.ts then
   * sends the turn and re-compacts with direct custom instructions when it settles).
   */
  onBeforeCompact(reason?: string): { cancel: boolean } {
    // A user-initiated /compact is a now request — never hijack it for a handoff
    // turn. Only automatic (threshold/overflow) compaction gets the intercept.
    if (reason === "manual") {
      return { cancel: false };
    }
    // Pi checks threshold compaction again after running a queued continuation,
    // before that continuation emits agent_settled. Keep cancelling until the
    // handoff turn's own later settle can observe its tool call, run the fallback
    // if needed, and trigger the one supported direct compact() path.
    if (this.handoffTurnArmedAtSettle != null) {
      return { cancel: true };
    }
    // We initiated this compaction after the handoff turn: let it proceed.
    if (this.recompacting) {
      this.recompacting = false;
      return { cancel: false };
    }
    // P3: only a *fresh* handoff (written inside the last window slice)
    // suppresses the turn; a stale one lets the intercept run so the summary
    // reflects current work.
    if (this.handoffIsFresh()) {
      return { cancel: false };
    }
    // Layer 3: cancel and give the agent one real turn to write its handoff.
    this.beginHandoffTurn("compaction imminent with no fresh handoff; running a handoff turn first");
    return { cancel: true };
  }

  /**
   * Called when the agent settles after we asked it for a handoff turn. Fires the
   * mechanical fallback only if the agent produced no handoff, then re-triggers
   * the compaction we cancelled. `wroteHandoff` is index.ts's observation of
   * whether a mission handoff tool call happened during the turn.
   *
   * The armed generation is consumed exactly once, and only by a later settle.
   * Even if the re-triggered compact() throws or its before_compact never fires,
   * we must not re-enter this path on subsequent settles (which would rotate a
   * fresh mechanical handoff every turn). recompacting arms the one legitimate
   * pass-through; onCompacted resets it.
   */
  onHandoffTurnSettled(wroteHandoff: boolean, lastUserMsg: string, settleSeq?: number): void {
    const observedSettle = this.observeSettleSequence(settleSeq);
    if (
      this.handoffTurnArmedAtSettle == null ||
      observedSettle <= this.handoffTurnArmedAtSettle
    ) {
      return;
    }
    this.handoffTurnArmedAtSettle = null;
    this.recompacting = true;
    if (!wroteHandoff) {
      this.actions.log("handoff turn wrote no handoff; using the mechanical fallback");
      const tokens = this.lastTokens != null ? String(this.lastTokens) : "unknown";
      this.actions.mechanicalHandoff(
        `compaction at ${tokens} tokens; last user message: ${truncate(lastUserMsg, 200)}`,
        "re-read the thread you were on and continue",
      );
    }
    // Re-trigger compaction; onBeforeCompact sees recompacting and lets it through.
    this.requestCompact(COMPACTION_INSTRUCTIONS);
  }

  /**
   * Observe both synchronous throws and asynchronous ctx.compact() rejections.
   * Pi rejects structurally uncompactable/no-op requests instead of emitting
   * session_compact, so the rejection path must release every cycle latch itself.
   */
  private requestCompact(customInstructions: string): void {
    let requested: void | Promise<void>;
    try {
      requested = this.actions.compact(customInstructions);
    } catch (err) {
      this.onCompactRejected(err);
      return;
    }
    void Promise.resolve(requested).catch((err: unknown) => {
      this.onCompactRejected(err);
    });
  }

  private onCompactRejected(err: unknown): void {
    const detail = String(err);
    if (isBenignCompactError(err)) {
      this.actions.log(`compact() no-op: ${detail}`);
    } else {
      this.actions.warn(`compact() failed: ${detail}`);
    }
    // A rejected compact never emits session_compact. Return to an idle cycle so
    // a later over-cap settle can retry. Keep handoff freshness: the handoff was
    // still written even though this particular compaction did not happen.
    this.capCompactRequested = false;
    this.recompacting = false;
    this.handoffTurnArmedAtSettle = null;
  }

  /** Called after a compaction fully completes: reset for the next cycle. */
  onCompacted(): void {
    this.handoffTurnArmedAtSettle = null;
    this.recompacting = false;
    this.handoffWritten = false;
    this.handoffTokens = null;
    this.lastNudgeFraction = null;
    this.capCompactRequested = false;
    this.lastTokens = null;
  }

  /**
   * Record that the agent called the mission handoff tool. Stamps the current
   * token count so the P3 freshness window can later tell whether this handoff is
   * still recent enough to suppress a pre-compaction turn.
   */
  noteHandoffWritten(): void {
    this.handoffWritten = true;
    this.handoffTokens = this.lastTokens;
  }

  /**
   * Record an explicit generation supplied by index.ts, or allocate one for a
   * direct unit-test call. Older/out-of-order values are retained as observations
   * but can never move the session's generation backwards.
   */
  private observeSettleSequence(settleSeq?: number): number {
    if (settleSeq == null) return this.beginAgentSettle();
    if (settleSeq > this.settleSequence) this.settleSequence = settleSeq;
    return settleSeq;
  }
}

export function isBenignCompactError(err: unknown): boolean {
  const detail = String(err);
  return detail.includes("Nothing to compact") || detail.includes("Already compacted");
}

function truncate(s: string, n: number): string {
  if (s.length <= n) return s;
  return s.slice(0, n);
}
