/**
 * TurnLedger transitions T1–T10 and the settle procedure (design §3.4). Pure —
 * no SDK, no I/O. Covers the fallback gating the M1 acceptance criteria pin:
 * covered vs bare-text, burst coalescing, kind exclusions, failed-reply-doesn't
 * -count, fallback-off, no-text, multi-group, internal exclusion, teardown
 * revert, and interrupt carry-forward.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { TurnLedger } from "../dist/ledger.js";

const opEnv = (chat, over = {}) => ({ source: "telegram", chat_id: chat, kind: "message", ...over });

test("T1 classification: operator / excluded kinds / no-envelope / unknown kind", () => {
  const l = new TurnLedger();
  assert.equal(l.register("u1", opEnv("1")), "operator");
  assert.equal(l.register("u2", opEnv("1", { kind: undefined })), "operator");
  assert.equal(l.register("u3", opEnv("1", { kind: "element_action" })), "operator");
  assert.equal(l.register("u4", opEnv("1", { kind: "schedule" })), "excluded");
  assert.equal(l.register("u5", opEnv("1", { kind: "notify" })), "excluded");
  assert.equal(l.register("u6", opEnv("1", { kind: "fleet" })), "excluded");
  assert.equal(l.register("u7", opEnv("1", { kind: "some_future_kind" })), "excluded");
  assert.equal(l.register("u8", null), "excluded");
  assert.equal(l.register("u9", { source: "telegram" }), "excluded"); // no chat_id
  assert.equal(l.register("u10", null, "handoff"), "internal");
});

test("operator turn answered by reply → NO fallback", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.onUserEcho("u1");
  l.onAssistantText(["some working text"]);
  l.onReplySuccess("telegram", "1");
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 0);
  assert.equal(r.coveredGroups, 1);
  assert.equal(l.stateOf("u1"), "SETTLED");
});

test("operator turn ending in bare text → exactly one fallback with the chat_id", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("42"));
  l.onUserEcho("u1");
  l.onAssistantText(["here is my answer"]);
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 1);
  assert.equal(r.fallbacks[0].chat_id, "42");
  assert.equal(r.fallbacks[0].source, "telegram");
  assert.equal(r.fallbacks[0].text, "here is my answer");
  assert.equal(r.fallbacks[0].outcome, "fallback");
});

test("burst coalescing: two messages one group → one fallback", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("7"));
  l.register("u2", opEnv("7"));
  l.onUserEcho("u1");
  l.onUserEcho("u2");
  l.onAssistantText(["combined reply"]);
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 1);
  assert.equal(r.multiTarget, false);
});

test("schedule/notify/fleet turn ending in bare text → NO fallback", () => {
  for (const kind of ["schedule", "notify", "fleet"]) {
    const l = new TurnLedger();
    l.register("u1", opEnv("1", { kind }));
    l.onUserEcho("u1");
    l.onAssistantText(["internal monologue"]);
    const r = l.onResult("success", true);
    assert.equal(r.fallbacks.length, 0, kind);
    assert.equal(r.excluded, 1, kind);
  }
});

test("reply that FAILED (isError) is not coverage → fallback still fires", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.onUserEcho("u1");
  l.onAssistantText(["answer"]);
  // onReplySuccess is NOT called (the PostToolUse hook skips isError replies).
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 1);
});

test("HOTLINE_SDK_FALLBACK off → no fallback, counted as a miss", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.onUserEcho("u1");
  l.onAssistantText(["answer"]);
  const r = l.onResult("success", false); // fallbackEnabled = false
  assert.equal(r.fallbacks.length, 0);
  assert.equal(r.noTextGroups, 1);
});

test("empty buffered text → no fallback (harness never invents words)", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.onUserEcho("u1");
  l.onAssistantText(["   ", ""]);
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 0);
  assert.equal(r.noTextGroups, 1);
});

test("multi-group turn → one fallback per group, multiTarget flagged", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("A"));
  l.register("u2", opEnv("B"));
  l.onUserEcho("u1");
  l.onUserEcho("u2");
  l.onAssistantText(["broadcast"]);
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 2);
  assert.equal(r.multiTarget, true);
  assert.deepEqual(r.fallbacks.map((f) => f.chat_id).sort(), ["A", "B"]);
});

test("error_during_execution result still fires fallback, tagged error_result", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.onUserEcho("u1");
  l.onAssistantText(["partial answer before the error"]);
  const r = l.onResult("error_during_execution", true);
  assert.equal(r.fallbacks.length, 1);
  assert.equal(r.fallbacks[0].outcome, "error_result");
});

test("internal handoff/compact turns settle excluded, never fallback", () => {
  const l = new TurnLedger();
  l.register("h1", null, "handoff");
  l.onUserEcho("h1");
  l.onAssistantText(["mechanical handoff written"]);
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 0);
  assert.equal(r.excluded, 1);
});

test("T3: unknown/absent uuid and replay echoes are ignored", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  assert.equal(l.onUserEcho(undefined), false);
  assert.equal(l.onUserEcho("never-registered"), false);
  assert.equal(l.onUserEcho("u1", true), false); // isReplay
  assert.equal(l.stateOf("u1"), "QUEUED");
  assert.equal(l.onUserEcho("u1"), true);
  assert.equal(l.stateOf("u1"), "DELIVERED");
});

test("T8 teardown revert: DELIVERED-not-SETTLED → QUEUED, re-echo works", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.onUserEcho("u1");
  assert.equal(l.stateOf("u1"), "DELIVERED");
  l.revertOnTeardown();
  assert.equal(l.stateOf("u1"), "QUEUED");
  // The retry re-delivers and re-echoes it; it can still settle.
  assert.equal(l.onUserEcho("u1"), true);
  l.onAssistantText(["answer after retry"]);
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 1);
});

test("T7 interrupt carry-forward: carried once, settled by the next turn", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.onUserEcho("u1");
  l.onAssistantText(["half an answer"]);
  l.markInterrupted();
  const r1 = l.onResult("success", true);
  // No fallback on the truncated half-answer; carried instead.
  assert.equal(r1.fallbacks.length, 0);
  assert.equal(r1.carried, 1);
  assert.equal(l.stateOf("u1"), "CARRIED");
  // Next turn (the steer's answer) covers it → settled, no fallback.
  l.onAssistantText(["the real answer after the steer"]);
  l.onReplySuccess("telegram", "1");
  const r2 = l.onResult("success", true);
  assert.equal(r2.fallbacks.length, 0);
  assert.equal(l.stateOf("u1"), "SETTLED");
});

test("T7: a carried entry that is NOT covered next turn fires a fallback", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.onUserEcho("u1");
  l.onAssistantText(["half"]);
  l.markInterrupted();
  l.onResult("success", true); // carried
  // Next turn produces text but no reply → the carried entry's group fallbacks.
  l.onAssistantText(["full answer, still no reply call"]);
  const r2 = l.onResult("success", true);
  assert.equal(r2.fallbacks.length, 1);
  assert.equal(r2.fallbacks[0].chat_id, "1");
});

test("source wildcard: reply with omitted source covers an envelope that has one", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1")); // source telegram
  l.onUserEcho("u1");
  l.onAssistantText(["answer"]);
  l.onReplySuccess(undefined, "1"); // single-provider: model omitted source
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 0);
});

test("source mismatch on a multi-provider box does NOT cover", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1", { source: "telegram" }));
  l.onUserEcho("u1");
  l.onAssistantText(["answer"]);
  l.onReplySuccess("discord", "1"); // wrong provider
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 1);
});

// ---- M1.1 re-arm: the 05:10 continuation leak -----------------------------

test("M1.1 re-arm: a continuation turn (no fresh delivery) still fires a fallback — the 05:10 leak", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("app"));
  l.onUserEcho("u1");
  l.onAssistantText(["Running — I'll report the fresh list."]);
  const r0 = l.onResult("success", true); // epoch 0: fallback fires
  assert.equal(r0.fallbacks.length, 1);
  assert.equal(r0.fallbacks[0].text, "Running — I'll report the fresh list.");
  assert.equal(l.awaitingCount(), 1, "conversation stays awaiting until a real reply");
  // Epoch 1: the SDK emits a SECOND result for the same inbound with NO fresh
  // user echo, trailing new assistant text, still no reply call. Pre-M1.1 this
  // slipped silently (no armed group). Now it is protected.
  l.onAssistantText(["Reran clean — pipeline survived the reboot fine."]);
  const r1 = l.onResult("success", true);
  assert.equal(r1.fallbacks.length, 1, "the continuation turn MUST be protected");
  assert.equal(r1.fallbacks[0].chat_id, "app");
  assert.equal(r1.fallbacks[0].text, "Reran clean — pipeline survived the reboot fine.");
});

test("M1.1 re-arm: a successful reply clears awaiting → no continuation fallback after", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.onUserEcho("u1");
  l.onAssistantText(["working"]);
  l.onReplySuccess("telegram", "1");
  const r0 = l.onResult("success", true);
  assert.equal(r0.fallbacks.length, 0);
  assert.equal(r0.replySatisfied.length, 1);
  assert.equal(l.awaitingCount(), 0);
  // A trailing continuation now has no awaiting conversation → nothing sent.
  l.onAssistantText(["some trailing note"]);
  const r1 = l.onResult("success", true);
  assert.equal(r1.fallbacks.length, 0);
});

test("M1.1 re-arm: an internal turn's tail text never leaks to an awaiting operator", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.onUserEcho("u1");
  l.onAssistantText(["first answer"]);
  l.onResult("success", true); // fallback fired, conversation still awaiting
  assert.equal(l.awaitingCount(), 1);
  // A mission handoff turn: internal delivery + its own text.
  l.register("h1", null, "handoff");
  l.onUserEcho("h1");
  l.onAssistantText(["mechanical handoff written"]);
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 0, "internal turn text must not reach the operator");
  assert.equal(r.excluded, 1);
  // A pure continuation AFTER an internal turn is also not attributed (the last
  // delivery was internal, not operator).
  l.onAssistantText(["internal tail"]);
  const r2 = l.onResult("success", true);
  assert.equal(r2.fallbacks.length, 0);
});

test("M1.1 re-arm: the same text is never sent twice for a conversation", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.onUserEcho("u1");
  l.onAssistantText(["identical text"]);
  const r0 = l.onResult("success", true);
  assert.equal(r0.fallbacks.length, 1);
  // A continuation repeating the exact buffered text → deduped, no re-send.
  l.onAssistantText(["identical text"]);
  const r1 = l.onResult("success", true);
  assert.equal(r1.fallbacks.length, 0);
  assert.equal(r1.misses.length, 1);
  assert.equal(r1.misses[0].reason, "dup_text");
});

test("M1.1 re-arm: a schedule turn while a conversation is awaiting does not leak to the operator", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.onUserEcho("u1");
  l.onAssistantText(["answer"]);
  l.onResult("success", true); // fallback, still awaiting
  // A schedule turn (excluded) with its own text must not go to the operator.
  l.register("s1", opEnv("1", { kind: "schedule" }));
  l.onUserEcho("s1");
  l.onAssistantText(["scheduled monologue"]);
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 0);
  assert.equal(r.excluded, 1);
});

test("M1.1 re-arm: continuation attributes to the last active conversation when several await", () => {
  const l = new TurnLedger();
  l.register("a1", opEnv("A"));
  l.onUserEcho("a1");
  l.onAssistantText(["ans A"]);
  l.onResult("success", true);
  l.register("b1", opEnv("B"));
  l.onUserEcho("b1");
  l.onAssistantText(["ans B"]);
  l.onResult("success", true);
  assert.equal(l.awaitingCount(), 2);
  // A continuation with no delivery → attributed to the most-recent (B).
  l.onAssistantText(["trailing detail"]);
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 1);
  assert.equal(r.fallbacks[0].chat_id, "B");
});

test("M1.1 re-arm: ambiguous continuation (last-active already replied, >1 others await) forwards nothing", () => {
  const l = new TurnLedger();
  l.register("a1", opEnv("A"));
  l.onUserEcho("a1");
  l.onAssistantText(["ans A"]);
  l.onResult("success", true); // A awaiting
  l.register("b1", opEnv("B"));
  l.onUserEcho("b1");
  l.onAssistantText(["ans B"]);
  l.onResult("success", true); // B awaiting
  l.register("c1", opEnv("C"));
  l.onUserEcho("c1");
  l.onAssistantText(["ans C"]);
  l.onReplySuccess("telegram", "C");
  l.onResult("success", true); // C covered+cleared, but C is last-active
  assert.equal(l.awaitingCount(), 2); // A and B
  // A continuation cannot be attributed to A or B → forward nothing, flag it.
  l.onAssistantText(["orphan tail"]);
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 0);
  assert.equal(r.ambiguous, true);
});

test("T10 unsettledCount reports queued/delivered/carried envelopes", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("1"));
  l.register("u2", opEnv("2"));
  l.onUserEcho("u1");
  assert.equal(l.unsettledCount(), 2); // u1 delivered, u2 queued
  l.onAssistantText(["a"]);
  l.onReplySuccess("telegram", "1");
  l.onResult("success", true);
  assert.equal(l.unsettledCount(), 1); // u2 still queued
});
