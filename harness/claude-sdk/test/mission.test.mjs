/**
 * Mission control handoff loop for claude-sdk (spec workstream 2) — the
 * applicable subset of pi's run-unit.mjs Part 7 ported against the reduced
 * `SdkMissionLoop`. All effects go through injected actions; no SDK, no queue,
 * no child process.
 *
 * The reduced loop drops pi's settle-generation fencing and cancel bookkeeping
 * (the AsyncQueue serializes our own injected turns and PreCompact cannot be
 * cancelled), so the cap cycle is a plain state machine:
 *   idle → handoffPending → compactQueued → (reset on compact_boundary) → idle.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  parseContextCap,
  effectiveCap,
  usageFraction,
  usageFromSdk,
  nudgeLine,
  SdkMissionLoop,
  COMPACTION_INSTRUCTIONS,
  HANDOFF_TURN_PROMPT,
} from "../dist/missionControl.js";

/** A recording actions harness. */
function harness(cap) {
  const events = { nudges: [], handoffTurns: [], compacts: [], mechanical: [] };
  const actions = {
    armNudge: (line) => events.nudges.push(line),
    sendHandoffTurn: (p) => events.handoffTurns.push(p),
    queueCompact: (ci) => events.compacts.push(ci),
    mechanicalHandoff: (s, n) => events.mechanical.push([s, n]),
    log: () => {},
    warn: () => {},
  };
  return { loop: new SdkMissionLoop(cap, actions), events };
}

// ---- Pure helpers ----------------------------------------------------------

test("parseContextCap unset/invalid → null, valid → int", () => {
  assert.equal(parseContextCap({}), null);
  assert.equal(parseContextCap({ HOTLINE_MC_CONTEXT_CAP: "nope" }), null);
  assert.equal(parseContextCap({ HOTLINE_MC_CONTEXT_CAP: "-5" }), null);
  assert.equal(parseContextCap({ HOTLINE_MC_CONTEXT_CAP: "0" }), null);
  assert.equal(parseContextCap({ HOTLINE_MC_CONTEXT_CAP: "120000" }), 120000);
});

test("usageFromSdk maps totalTokens/maxTokens, missing → null/0", () => {
  assert.deepEqual(usageFromSdk({ totalTokens: 100, maxTokens: 200 }), { tokens: 100, contextWindow: 200 });
  assert.deepEqual(usageFromSdk({}), { tokens: null, contextWindow: 0 });
  assert.deepEqual(usageFromSdk({ totalTokens: 5 }), { tokens: 5, contextWindow: 0 });
});

test("effectiveCap prefers the soft cap, falls back to the window", () => {
  assert.equal(effectiveCap({ tokens: 1, contextWindow: 200000 }, 100000), 100000);
  assert.equal(effectiveCap({ tokens: 1, contextWindow: 200000 }, null), 200000);
  assert.equal(effectiveCap({ tokens: 1, contextWindow: 0 }, null), null);
});

test("usageFraction handles null tokens and computes against the cap", () => {
  assert.equal(usageFraction({ tokens: null, contextWindow: 200000 }, null), null);
  assert.ok(Math.abs(usageFraction({ tokens: 90000, contextWindow: 200000 }, 100000) - 0.9) < 1e-9);
});

test("the four constants are copied byte-for-byte from pi", () => {
  assert.ok(COMPACTION_INSTRUCTIONS.includes("Append exactly this final block"));
  assert.ok(COMPACTION_INSTRUCTIONS.includes("MISSION HANDOFF\nDoing:"));
  assert.ok(HANDOFF_TURN_PROMPT.startsWith("[mission-control]"));
  assert.ok(nudgeLine(82).includes("82% of its working budget"));
});

// ---- Layer 1: nudge --------------------------------------------------------

test("nudge arms at 80%, re-arms only after +10% growth", () => {
  const { loop, events } = harness(100000);
  loop.onResult({ tokens: 70000, contextWindow: 200000 });
  assert.equal(events.nudges.length, 0, "no nudge below 80%");
  loop.onResult({ tokens: 82000, contextWindow: 200000 });
  assert.equal(events.nudges.length, 1, "nudge arms at 82%");
  loop.onResult({ tokens: 85000, contextWindow: 200000 });
  assert.equal(events.nudges.length, 1, "no re-nudge inside the 10% step");
  loop.onResult({ tokens: 93000, contextWindow: 200000 });
  assert.equal(events.nudges.length, 2, "re-nudge after +10% growth");
});

test("tokens-null resets the nudge gate so it re-arms", () => {
  const { loop, events } = harness(100000);
  loop.onResult({ tokens: 90000, contextWindow: 200000 });
  assert.equal(events.nudges.length, 1);
  loop.onResult({ tokens: null, contextWindow: 200000 });
  loop.onResult({ tokens: 82000, contextWindow: 200000 });
  assert.equal(events.nudges.length, 2, "the nudge re-arms after a tokens-null reset");
});

test("no cap: the nudge still fires against the window, no compaction", () => {
  const { loop, events } = harness(null);
  assert.equal(loop.hasCap(), false);
  loop.onResult({ tokens: 170000, contextWindow: 200000 });
  assert.equal(events.nudges.length, 1, "nudge fires against the window with no cap");
  assert.equal(events.compacts.length, 0);
  assert.equal(events.handoffTurns.length, 0);
});

// ---- The cap cycle (P2-B) --------------------------------------------------

test("cap crossing runs a handoff turn exactly once, then falls back + compacts", () => {
  const { loop, events } = harness(100000);
  loop.noteUserPrompt("some real work");
  loop.onResult({ tokens: 99000, contextWindow: 200000 });
  assert.equal(events.handoffTurns.length, 0, "no handoff turn below the cap");
  assert.equal(events.compacts.length, 0, "no compact below the cap");
  loop.onResult({ tokens: 101000, contextWindow: 200000 });
  assert.equal(events.handoffTurns.length, 1, "cap crossed sends a handoff turn first");
  assert.equal(events.compacts.length, 0, "compaction waits for the turn to settle");
  // The handoff turn settles (next result); the model wrote nothing.
  loop.onResult({ tokens: 102000, contextWindow: 200000 });
  assert.equal(events.handoffTurns.length, 1, "the handoff turn is not re-sent");
  assert.equal(events.mechanical.length, 1, "the mechanical fallback fires when nothing was written");
  assert.equal(events.compacts.length, 1, "compaction follows the handoff turn's settle");
  assert.ok(events.mechanical[0][0].includes("102000 tokens"), "fallback state carries the token count");
  assert.ok(events.mechanical[0][0].includes("some real work"), "fallback state carries the last user message");
  assert.equal(events.mechanical[0][1], "re-read the thread you were on and continue");
});

test("cap path: a handoff written during the turn skips the mechanical fallback", () => {
  const { loop, events } = harness(100000);
  loop.onResult({ tokens: 101000, contextWindow: 200000 });
  assert.equal(events.handoffTurns.length, 1);
  loop.noteHandoffWritten(); // the model wrote its handoff during the turn
  loop.onResult({ tokens: 102000, contextWindow: 200000 });
  assert.equal(events.mechanical.length, 0, "no fallback when the agent wrote a handoff");
  assert.equal(events.compacts.length, 1, "still compacts after the turn");
});

test("cap path: a FRESH handoff already on disk compacts directly, no turn", () => {
  const { loop, events } = harness(100000);
  loop.onResult({ tokens: 96000, contextWindow: 200000 }); // inside the last 25% of the cap
  loop.noteHandoffWritten();
  loop.onResult({ tokens: 101000, contextWindow: 200000 });
  assert.equal(events.handoffTurns.length, 0, "fresh handoff skips the handoff turn");
  assert.equal(events.compacts.length, 1, "fresh handoff compacts directly");
});

test("cap path: a STALE handoff still runs the handoff turn", () => {
  const { loop, events } = harness(100000);
  loop.onResult({ tokens: 40000, contextWindow: 200000 }); // handoff far from the cap
  loop.noteHandoffWritten();
  loop.onResult({ tokens: 101000, contextWindow: 200000 });
  assert.equal(events.handoffTurns.length, 1, "stale handoff still triggers the turn");
  assert.equal(events.compacts.length, 0, "compaction waits for the stale-handoff turn");
});

test("a settle while compactQueued never re-fires mechanical/compact in the same call", () => {
  const { loop, events } = harness(100000);
  loop.onResult({ tokens: 101000, contextWindow: 200000 }); // handoff turn
  loop.onResult({ tokens: 102000, contextWindow: 200000 }); // settle → mechanical + compact
  assert.equal(events.mechanical.length, 1);
  assert.equal(events.compacts.length, 1);
  // The next settle arriving while still compactQueued is the /compact turn's own
  // no-op settle: it resets to idle but must not re-run mechanical/compact within
  // that call (the re-arm, if any, is deferred to the NEXT over-cap settle).
  loop.onResult({ tokens: 103000, contextWindow: 200000 });
  assert.equal(events.mechanical.length, 1, "no repeat mechanical inside the reset settle");
  assert.equal(events.compacts.length, 1, "no repeat compaction inside the reset settle");
});

test("compact_boundary resets the cycle so a later cap crossing runs again", () => {
  const { loop, events } = harness(100000);
  loop.onResult({ tokens: 101000, contextWindow: 200000 });
  assert.equal(events.handoffTurns.length, 1);
  loop.onCompactBoundary();
  loop.onResult({ tokens: 101000, contextWindow: 200000 });
  assert.equal(events.handoffTurns.length, 2, "the cycle re-arms after compact_boundary");
});

// ---- No-op /compact detection (the wedge fix) ------------------------------

test("a no-op /compact (settle with no compact_boundary) resets the cycle to idle", () => {
  const { loop, events } = harness(100000);
  // Cap crossed → handoff turn → settle → mechanical + /compact queued.
  loop.onResult({ tokens: 101000, contextWindow: 200000 }); // handoff turn
  loop.onResult({ tokens: 102000, contextWindow: 200000 }); // settle → compact queued
  assert.equal(events.compacts.length, 1, "the /compact was queued");
  // The CLI short-circuits: the /compact turn settles WITHOUT a compact_boundary.
  // This is the wedge — previously the loop stayed compactQueued forever. Now it
  // must reset to idle so a later cap crossing can re-arm.
  loop.onResult({ tokens: 103000, contextWindow: 200000 }); // /compact no-op settle
  // Prove idle by crossing the cap again: the cycle re-fires (handoff turn #2).
  loop.onResult({ tokens: 104000, contextWindow: 200000 });
  assert.equal(events.handoffTurns.length, 2, "the cycle re-arms after a no-op /compact reset");
});

test("a real compact_boundary still resets normally (no premature no-op reset)", () => {
  const { loop, events } = harness(100000);
  loop.onResult({ tokens: 101000, contextWindow: 200000 }); // handoff turn
  loop.onResult({ tokens: 102000, contextWindow: 200000 }); // settle → compact queued
  assert.equal(events.compacts.length, 1);
  // A real compaction: compact_boundary fires (resets to idle) BEFORE the
  // /compact turn's own result. The subsequent post-compact settle must not
  // re-enter the no-op path — it lands in idle and re-evaluates the cap fresh.
  loop.onCompactBoundary();
  loop.onResult({ tokens: 30000, contextWindow: 200000 }); // post-compact settle, under cap
  assert.equal(events.compacts.length, 1, "no second compact from the post-boundary settle");
  assert.equal(events.handoffTurns.length, 1, "no spurious handoff turn under the cap");
});

test("the no-op reset re-arms cap enforcement on the very next over-cap settle", () => {
  const { loop, events } = harness(100000);
  loop.onResult({ tokens: 101000, contextWindow: 200000 }); // handoff turn #1
  loop.onResult({ tokens: 102000, contextWindow: 200000 }); // settle → compact #1 queued
  loop.onResult({ tokens: 103000, contextWindow: 200000 }); // /compact no-op → reset idle
  assert.equal(events.compacts.length, 1, "only the first /compact so far");
  assert.equal(events.handoffTurns.length, 1, "no re-arm inside the no-op settle itself");
  // Next over-cap settle re-arms the full cap cycle a second time.
  loop.onResult({ tokens: 105000, contextWindow: 200000 }); // handoff turn #2
  loop.onResult({ tokens: 106000, contextWindow: 200000 }); // settle → compact #2 queued
  assert.equal(events.handoffTurns.length, 2, "cap enforcement re-armed after the no-op reset");
  assert.equal(events.compacts.length, 2, "a second /compact is queued after the reset");
});

// ---- The CLI-auto divergence: PreCompact (spec §2 resolution) --------------

test("PreCompact(manual) is left alone — a user /compact is a now request", () => {
  const { loop } = harness(100000);
  assert.deepEqual(loop.onPreCompact("manual"), { mechanical: null });
});

test("PreCompact(auto) with no fresh handoff returns the mechanical fallback", () => {
  const { loop } = harness(null); // no cap so onResult only records usage
  loop.noteUserPrompt("did some work here");
  loop.onResult({ tokens: 150000, contextWindow: 200000 });
  const { mechanical } = loop.onPreCompact("auto");
  assert.ok(mechanical, "auto + absent handoff → mechanical");
  assert.ok(mechanical.state.includes("150000 tokens"));
  assert.ok(mechanical.state.includes("did some work here"));
  assert.equal(mechanical.next, "re-read the thread you were on and continue");
});

test("PreCompact(auto) with a fresh handoff runs no mechanical fallback", () => {
  const { loop } = harness(null);
  loop.onResult({ tokens: 150000, contextWindow: 200000 }); // effCap = window 200000
  loop.noteHandoffWritten(); // 150000 >= 0.75 * 200000 → fresh
  assert.deepEqual(loop.onPreCompact("auto"), { mechanical: null });
});

test("PreCompact(auto) stamps freshness so a second hook does not double-fire", () => {
  const { loop } = harness(null);
  loop.onResult({ tokens: 150000, contextWindow: 200000 });
  const first = loop.onPreCompact("auto");
  assert.ok(first.mechanical, "first auto hook fires the mechanical handoff");
  const second = loop.onPreCompact("auto");
  assert.equal(second.mechanical, null, "the second hook sees the just-written handoff as fresh");
});
