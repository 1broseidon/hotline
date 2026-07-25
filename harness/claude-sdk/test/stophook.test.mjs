/**
 * M1.1 Stop-hook enforcement predicate (design §M1.1). The Stop hook installed
 * in index.ts consumes `ledger.activeUncoveredGroups()` + the epoch-scoped
 * block counter; these tests pin that predicate directly (pure, no SDK). The
 * hook blocks the turn's end while the ledger says the operator is still owed a
 * reply, caps at 2 blocks per turn, then lets the fallback lane deliver.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { TurnLedger } from "../dist/ledger.js";

const opEnv = (chat, over = {}) => ({ source: "telegram", chat_id: chat, kind: "message", ...over });

test("owed reply while the turn is open → non-empty (Stop hook blocks)", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("9"));
  l.onUserEcho("u1");
  l.onAssistantText(["bare text, no reply call"]);
  const owed = l.activeUncoveredGroups();
  assert.equal(owed.length, 1);
  assert.equal(owed[0].chat_id, "9");
});

test("covered by a successful reply → empty (Stop hook lets it end)", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("9"));
  l.onUserEcho("u1");
  l.onAssistantText(["answer"]);
  l.onReplySuccess("telegram", "9");
  assert.equal(l.activeUncoveredGroups().length, 0);
});

test("internal (mission handoff) turn → empty (never block, never leak internal)", () => {
  const l = new TurnLedger();
  l.register("h1", null, "handoff");
  l.onUserEcho("h1");
  l.onAssistantText(["handoff text"]);
  assert.equal(l.activeUncoveredGroups().length, 0);
});

test("excluded (schedule) turn → empty (no enforcement on non-operator turns)", () => {
  const l = new TurnLedger();
  l.register("s1", opEnv("9", { kind: "schedule" }));
  l.onUserEcho("s1");
  l.onAssistantText(["scheduled monologue"]);
  assert.equal(l.activeUncoveredGroups().length, 0);
});

test("continuation of an awaiting conversation → non-empty (blocks the leak turn)", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("9"));
  l.onUserEcho("u1");
  l.onAssistantText(["first"]);
  l.onResult("success", true); // fallback fired, conversation still awaiting
  // The follow-on turn (the 05:10 continuation-leak shape): open turn, new text, no echo.
  l.onAssistantText(["second, still no reply"]);
  assert.equal(l.activeUncoveredGroups().length, 1);
});

test("idle turn with no text → empty (an awaiting conversation is not spammed)", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("9"));
  l.onUserEcho("u1");
  l.onAssistantText(["first"]);
  l.onResult("success", true); // awaiting
  // A later turn that produces no operator text (e.g. tool-only) → nothing owed.
  assert.equal(l.activeUncoveredGroups().length, 0);
});

test("block cap is epoch-scoped: counts up within a turn, resets on settle", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("9"));
  l.onUserEcho("u1");
  l.onAssistantText(["x"]);
  assert.equal(l.stopBlockCount, 0);
  l.noteStopBlock();
  assert.equal(l.stopBlockCount, 1);
  l.noteStopBlock();
  assert.equal(l.stopBlockCount, 2); // at the cap; the hook now lets it end
  l.onResult("success", true); // fallback lane delivers after 2 blocks
  assert.equal(l.stopBlockCount, 0, "reset for the next turn");
});

test("enforcement and the fallback lane agree: the same turn is owed and fired", () => {
  const l = new TurnLedger();
  l.register("u1", opEnv("9"));
  l.onUserEcho("u1");
  l.onAssistantText(["answer the operator never sees without help"]);
  // Stop hook would block here (owed non-empty)...
  assert.equal(l.activeUncoveredGroups().length, 1);
  // ...and after the block cap, the settle fires the lane for the same group.
  const r = l.onResult("success", true);
  assert.equal(r.fallbacks.length, 1);
  assert.equal(r.fallbacks[0].chat_id, "9");
});
