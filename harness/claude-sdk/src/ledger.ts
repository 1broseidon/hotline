/**
 * The turn ledger + attribution state machine (design §3.4). PURE: no SDK
 * imports, no I/O, fully unit-tested. It is the single source of truth for
 * which SDK turn consumed which inbound envelope and whether that turn answered
 * the channel — the machinery behind M1's delivery guarantee and M1.1's
 * stop-hook enforcement.
 *
 * The transitions T1–T10 in the design map to the methods below. The settle
 * procedure (T6) returns a plan the fallback executor runs; the ledger itself
 * never touches the wire.
 *
 * M1.1 (2026-07-24, the 05:10 continuation-leak fix): delivery is no longer
 * arm-once-per-envelope. An operator conversation `(source, chat_id)` becomes
 * "awaiting delivery" the moment an operator envelope is consumed, and STAYS
 * awaiting — across epochs — until a SUCCESSFUL `reply` lands for it. A
 * continuation turn (the SDK emits a second `result` for the same inbound with
 * no fresh user echo) therefore still has its buffered text protected, and the
 * Stop-hook enforcement reads the same `activeUncoveredGroups()` predicate the
 * settle uses, so the locked door and the safety net agree on what "unanswered"
 * means. See T11 below.
 */

import type { Envelope } from "./envelope.js";

export type Classification = "operator" | "excluded" | "internal";

/** Internal (harness-injected) turn markers: mission handoff / `/compact`. */
export type InternalKind = "handoff" | "compact";

export type EnvelopeState = "QUEUED" | "DELIVERED" | "SETTLED" | "CARRIED";

export type SettleOutcome =
  | "covered"
  | "fallback"
  | "no_text"
  | "excluded"
  | "error_result";

/** Why an uncovered operator group settled without a fallback send. */
export type MissReason = "fallback_off" | "no_text" | "dup_text";

export interface LedgerEntry {
  uuid: string;
  env: Envelope | null;
  internal?: InternalKind;
  klass: Classification;
  state: EnvelopeState;
  /** Registration order — used for pull-window attribution and lease math. */
  order: number;
  /** The epoch the entry was delivered into (T2). */
  epoch?: number;
  /** How many times this entry has been carried across an interrupt (T7). */
  carryCount: number;
}

/** One fallback the executor must send for an uncovered operator group. */
export interface FallbackAction {
  source?: string;
  chat_id: string;
  text: string;
  epoch: number;
  outcome: "fallback" | "error_result";
}

/** A settled operator group, for the one-line-per-outcome settle log (goal 3). */
export interface GroupRef {
  source?: string;
  chat_id: string;
}

export interface Miss extends GroupRef {
  reason: MissReason;
}

export interface SettleResult {
  fallbacks: FallbackAction[];
  /** Distinct (source,chat_id) operator groups that reply() covered this turn. */
  coveredGroups: number;
  /** Operator groups settled with no send (see `misses` for the why breakdown). */
  noTextGroups: number;
  /** Excluded/internal envelopes settled this turn. */
  excluded: number;
  /** Operator entries carried to the next epoch (interrupt). */
  carried: number;
  /** True when the settled turn had >1 distinct operator group. */
  multiTarget: boolean;
  /** The result subtype recorded (telemetry only; does not gate the lane). */
  subtype: string;
  /** Groups a successful reply satisfied (cleared from awaiting) — for the log. */
  replySatisfied: GroupRef[];
  /** Uncovered groups that settled without a send, with the reason — for the log. */
  misses: Miss[];
  /** A continuation turn produced text while >1 conversation was awaiting and
   * none was delivered this turn: we cannot attribute it, so we forward nothing
   * and log it (never cross-talk). */
  ambiguous: boolean;
}

interface CoverKey {
  source?: string;
  chat_id: string;
}

function groupKey(env: Envelope): string {
  return `${env.source ?? ""} ${env.chat_id ?? ""}`;
}

/**
 * A reply covers an operator group when its chat_id matches and the sources are
 * compatible: an omitted source on either side is a wildcard (single-provider
 * boxes let the model omit source; the Go router defaults it to the sole
 * provider, so the harness cannot reconstruct it and must not require a match).
 */
function coversGroup(cover: CoverKey, env: Envelope): boolean {
  if (cover.chat_id !== (env.chat_id ?? "")) return false;
  const cs = cover.source ?? "";
  const es = env.source ?? "";
  if (cs === "" || es === "") return true;
  return cs === es;
}

interface Accumulator {
  open: boolean;
  textBuf: string[];
  covered: CoverKey[];
  delivered: LedgerEntry[];
  interrupted: boolean;
  /** M1.1: Stop-hook blocks issued this turn (epoch-scoped, cap 2). */
  stopBlocks: number;
}

/** An operator conversation still awaiting a SUCCESSFUL model reply. Persists
 * across epochs until a reply covers it (M1.1 re-arm). `lastSentText` dedups
 * the fallback lane so it never sends the same text twice. */
interface AwaitingGroup {
  env: Envelope;
  lastSentText?: string;
}

/** An operator group the current open turn's text/delivery pertains to, with
 * its delivered-this-epoch operator entries (empty for a pure continuation). */
interface ActiveGroup {
  key: string;
  env: Envelope;
  entries: LedgerEntry[];
}

function classify(env: Envelope | null, internal?: InternalKind): Classification {
  if (internal) return "internal";
  if (!env) return "excluded";
  if (!env.chat_id) return "excluded";
  const kind = env.kind;
  // operator-triggered: absent/empty kind, plain message, or an element tap.
  if (kind === undefined || kind === "" || kind === "message" || kind === "element_action") {
    return "operator";
  }
  // schedule / notify / fleet — and any UNKNOWN future kind — are excluded:
  // a new kind must opt in; silence is safer than leaking monologue to a peer.
  return "excluded";
}

export class TurnLedger {
  private entries = new Map<string, LedgerEntry>();
  private order = 0;
  epoch = 0;
  private acc: Accumulator = TurnLedger.freshAcc();
  /** Entries carried from a prior interrupted settle, folded into the next. */
  private carryOver: LedgerEntry[] = [];
  /** M1.1: operator conversations awaiting a successful reply (keyed by
   * (source,chat_id)). Persists across epochs — the re-arm that fixes the
   * 05:10 continuation leak. */
  private awaiting = new Map<string, AwaitingGroup>();
  /** The most recent operator group delivered — the continuation-attribution
   * key when a turn produces text with no fresh delivery. */
  private lastActiveGroup: string | null = null;
  /** Whether the most recent delivery (any epoch) was an operator envelope.
   * Guards continuation attribution so an internal turn's tail text can never
   * be forwarded to the operator. */
  private lastDeliveredWasOperator = false;

  private static freshAcc(): Accumulator {
    return { open: false, textBuf: [], covered: [], delivered: [], interrupted: false, stopBlocks: 0 };
  }

  private armAwaiting(key: string, env: Envelope, sentText?: string): void {
    const cur = this.awaiting.get(key);
    this.awaiting.set(key, { env, lastSentText: sentText ?? cur?.lastSentText });
  }

  /** T1 — register an enqueued message. Returns its classification so the
   * caller can stamp priority and persist last-operator. */
  register(uuid: string, env: Envelope | null, internal?: InternalKind): Classification {
    const klass = classify(env, internal);
    this.entries.set(uuid, {
      uuid,
      env,
      internal,
      klass,
      state: "QUEUED",
      order: this.order++,
      carryCount: 0,
    });
    return klass;
  }

  /** The classification of an already-registered uuid, or null if unknown. */
  classOf(uuid: string): Classification | null {
    return this.entries.get(uuid)?.klass ?? null;
  }

  /** T2/T3 — a user message echoed back on the stream. Unknown/absent uuids and
   * replay messages are ignored (T3). Returns true when it was a fresh
   * delivery (T2). Arms the operator conversation as awaiting-delivery. */
  onUserEcho(uuid: string | undefined, isReplay = false): boolean {
    if (isReplay || !uuid) return false;
    const e = this.entries.get(uuid);
    if (!e || e.state !== "QUEUED") return false; // unknown or already delivered
    if (!this.acc.open) this.acc.open = true;
    e.state = "DELIVERED";
    e.epoch = this.epoch;
    this.acc.delivered.push(e);
    if (e.klass === "operator" && e.env) {
      const key = groupKey(e.env);
      this.armAwaiting(key, e.env);
      this.lastActiveGroup = key;
    }
    return true;
  }

  /** T4 — assistant text blocks. Opens the turn; buffers every text block. */
  onAssistantText(texts: string[]): void {
    if (!this.acc.open) this.acc.open = true;
    for (const t of texts) if (typeof t === "string" && t !== "") this.acc.textBuf.push(t);
  }

  /** T5 — a reply tool call that SUCCEEDED (PostToolUse, isError !== true). */
  onReplySuccess(source: string | undefined, chat_id: string | undefined): void {
    if (!chat_id) return;
    this.acc.covered.push({ source: source || undefined, chat_id });
  }

  /** T7 — we interrupted this turn. */
  markInterrupted(): void {
    this.acc.interrupted = true;
  }

  /** Whether a turn is currently open (an operator/steer decision input). */
  get turnOpen(): boolean {
    return this.acc.open;
  }

  // ---- M1.1 Stop-hook enforcement (T11) ----------------------------------

  /** Blocks issued this turn (epoch-scoped; the cap is enforced by the caller). */
  get stopBlockCount(): number {
    return this.acc.stopBlocks;
  }

  /** Record one Stop-hook block for the current turn. */
  noteStopBlock(): void {
    this.acc.stopBlocks++;
  }

  /**
   * T11 — the operator groups this open turn owes a reply, read-only. The Stop
   * hook blocks the turn's end when this is non-empty; the settle procedure
   * fires the fallback lane for the same set. Empty ⇒ the turn is not
   * operator-relevant (internal/excluded/idle) or every relevant group was
   * already covered by a successful reply — either way, let it end.
   */
  activeUncoveredGroups(): GroupRef[] {
    const { groups } = this.computeActiveGroups();
    const out: GroupRef[] = [];
    for (const g of groups) {
      if (this.acc.covered.some((c) => coversGroup(c, g.env))) continue;
      out.push({ source: g.env.source, chat_id: g.env.chat_id as string });
    }
    return out;
  }

  /**
   * The operator groups the current open turn's text/delivery pertains to
   * (read-only). Delivered-this-epoch operator groups win; otherwise a pure
   * continuation (nothing delivered this turn, but text produced while a
   * conversation is still awaiting a real reply) attributes to the last active
   * conversation. Internal/excluded-only turns and ambiguous multi-conversation
   * continuations attribute to nothing.
   */
  private computeActiveGroups(): { groups: ActiveGroup[]; ambiguous: boolean } {
    const acc = this.acc;
    const operatorEntries = [...acc.delivered, ...this.carryOver].filter((e) => e.klass === "operator");
    const delivered = new Map<string, ActiveGroup>();
    for (const e of operatorEntries) {
      const env = e.env as Envelope;
      const key = groupKey(env);
      const g = delivered.get(key);
      if (g) g.entries.push(e);
      else delivered.set(key, { key, env, entries: [e] });
    }
    if (delivered.size > 0) return { groups: [...delivered.values()], ambiguous: false };

    // Pure continuation: nothing delivered this epoch (not even internal), but
    // the model produced text while an operator conversation is still awaiting a
    // real reply. This is the 05:10 continuation-leak case.
    const text = acc.textBuf.join("\n").trim();
    if (text === "" || acc.delivered.length > 0 || !this.lastDeliveredWasOperator || this.awaiting.size === 0) {
      return { groups: [], ambiguous: false };
    }
    let key: string | null = null;
    if (this.lastActiveGroup && this.awaiting.has(this.lastActiveGroup)) key = this.lastActiveGroup;
    else if (this.awaiting.size === 1) key = [...this.awaiting.keys()][0];
    if (!key) return { groups: [], ambiguous: true }; // >1 awaiting, cannot attribute
    return { groups: [{ key, env: this.awaiting.get(key)!.env, entries: [] }], ambiguous: false };
  }

  /** T6 — settle the open turn. Returns the fallback plan + telemetry, then
   * advances the epoch and resets the accumulator. */
  onResult(subtype: string, fallbackEnabled: boolean): SettleResult {
    const acc = this.acc;
    const text = acc.textBuf.join("\n").trim();
    const isError = /^error/i.test(subtype);

    // State bookkeeping: excluded/internal entries settle immediately.
    const all = [...acc.delivered, ...this.carryOver];
    const operatorEntries: LedgerEntry[] = [];
    let excluded = 0;
    for (const e of all) {
      if (e.klass === "operator") operatorEntries.push(e);
      else {
        e.state = "SETTLED";
        excluded++;
      }
    }

    // A successful reply satisfies its conversation everywhere: clear awaiting
    // for every covered group, even one not touched this epoch.
    const replySatisfied: GroupRef[] = [];
    if (acc.covered.length > 0) {
      for (const [key, g] of [...this.awaiting.entries()]) {
        if (acc.covered.some((c) => coversGroup(c, g.env))) {
          this.awaiting.delete(key);
          replySatisfied.push({ source: g.env.source, chat_id: g.env.chat_id as string });
        }
      }
    }

    // Delivered-this-epoch operator groups (state bookkeeping + multiTarget).
    const deliveredGroups = new Map<string, LedgerEntry[]>();
    for (const e of operatorEntries) {
      const key = groupKey(e.env as Envelope);
      const arr = deliveredGroups.get(key);
      if (arr) arr.push(e);
      else deliveredGroups.set(key, [e]);
    }

    const { groups: activeGroups, ambiguous } = this.computeActiveGroups();

    const fallbacks: FallbackAction[] = [];
    const newCarryOver: LedgerEntry[] = [];
    const misses: Miss[] = [];
    let coveredGroups = 0;
    let noTextGroups = 0;
    let carried = 0;

    for (const g of activeGroups) {
      const entries = deliveredGroups.get(g.key) ?? [];
      const ref: GroupRef = { source: g.env.source, chat_id: g.env.chat_id as string };
      const isCovered = acc.covered.some((c) => coversGroup(c, g.env));
      if (isCovered) {
        for (const e of entries) e.state = "SETTLED";
        this.awaiting.delete(g.key);
        coveredGroups++;
        continue;
      }
      if (!fallbackEnabled) {
        for (const e of entries) e.state = "SETTLED";
        this.armAwaiting(g.key, g.env); // still undelivered — stay awaiting
        noTextGroups++;
        misses.push({ ...ref, reason: "fallback_off" });
        continue;
      }
      if (text === "") {
        for (const e of entries) e.state = "SETTLED";
        this.armAwaiting(g.key, g.env);
        noTextGroups++;
        misses.push({ ...ref, reason: "no_text" });
        continue;
      }
      if (acc.interrupted && entries.length > 0) {
        // Carry uncovered operator entries once; a second interrupt settles
        // them as error_result (no fallback on a truncated half-answer).
        for (const e of entries) {
          if (e.carryCount < 1) {
            e.carryCount++;
            e.state = "CARRIED";
            newCarryOver.push(e);
            carried++;
          } else {
            e.state = "SETTLED";
          }
        }
        continue;
      }
      const existing = this.awaiting.get(g.key);
      if (existing && existing.lastSentText === text) {
        // Never send the same text twice for a conversation.
        for (const e of entries) e.state = "SETTLED";
        noTextGroups++;
        misses.push({ ...ref, reason: "dup_text" });
        continue;
      }
      // Fallback fires. The conversation STAYS awaiting (the model still owes a
      // real reply); lastSentText dedups a repeat of this exact text.
      for (const e of entries) e.state = "SETTLED";
      this.armAwaiting(g.key, g.env, text);
      fallbacks.push({
        source: g.env.source,
        chat_id: g.env.chat_id as string,
        text,
        epoch: this.epoch,
        outcome: isError ? "error_result" : "fallback",
      });
    }

    // Track whether the latest delivery was operator, for the next epoch's
    // continuation guard. An empty turn (no delivery) preserves the prior value.
    if (deliveredGroups.size > 0) this.lastDeliveredWasOperator = true;
    else if (acc.delivered.length > 0) this.lastDeliveredWasOperator = false;

    const result: SettleResult = {
      fallbacks,
      coveredGroups,
      noTextGroups,
      excluded,
      carried,
      multiTarget: deliveredGroups.size > 1,
      subtype,
      replySatisfied,
      misses,
      ambiguous,
    };

    this.carryOver = newCarryOver;
    this.epoch++;
    this.acc = TurnLedger.freshAcc();
    return result;
  }

  /** T8 — attempt teardown (retry-without-resume / stream throw). Every
   * DELIVERED-not-SETTLED envelope reverts to QUEUED so the next attempt
   * re-delivers and re-echoes it; the accumulator and carry-over reset. The
   * awaiting map is intentionally preserved: the re-delivery re-arms the same
   * group, and a conversation still owed a reply must not be forgotten. */
  revertOnTeardown(): void {
    for (const e of this.acc.delivered) {
      if (e.state === "DELIVERED") {
        e.state = "QUEUED";
        e.epoch = undefined;
      }
    }
    for (const e of this.carryOver) {
      if (e.state === "CARRIED") {
        e.state = "QUEUED";
        e.epoch = undefined;
        e.carryCount = 0;
      }
    }
    this.carryOver = [];
    this.acc = TurnLedger.freshAcc();
  }

  /** T10 — queue close (shutdown). No fallback on the abort path; report the
   * count of envelopes that never settled. */
  unsettledCount(): number {
    let n = 0;
    for (const e of this.entries.values()) {
      if (e.state === "QUEUED" || e.state === "DELIVERED" || e.state === "CARRIED") n++;
    }
    return n;
  }

  /** Diagnostics/tests: conversations still awaiting a successful reply. */
  awaitingCount(): number {
    return this.awaiting.size;
  }

  /** Diagnostics/tests: the current state of a registered uuid. */
  stateOf(uuid: string): EnvelopeState | null {
    return this.entries.get(uuid)?.state ?? null;
  }
}
