/**
 * The SDK hot-model apply handler (sdkapply.ts): guard hit/miss, clear,
 * setModel throw, no_session, malformed drops, and the mirror-then-announce
 * ordering (onApplied strictly before the ok result). The SDK control surface
 * is mocked — the contract under test is ours, not the CLI's.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { createSdkApplyHandler, firstLine, idExtendsAtBoundary, matchModel } from "../dist/sdkapply.js";

const silentLog = { info() {}, warn() {}, error() {} };

/** A deps harness recording every observable in call order. */
function makeHarness({ query, getSession } = {}) {
  const events = [];
  const handler = createSdkApplyHandler({
    getSession:
      getSession ?? (() => (query ? { query, generation: 1 } : null)),
    notifyResult: (result) => events.push({ kind: "result", result }),
    onApplied: (applied) => events.push({ kind: "applied", ...applied }),
    log: silentLog,
  });
  return { handler, events };
}

/** A mock Query whose model/effort control behavior is scriptable. */
function makeQuery({ models = [], setModelError = null, applyFlagError = null, maxTokError = null } = {}) {
  const calls = { supportedModels: 0, setModel: [], applyFlagSettings: [], setMaxThinkingTokens: [] };
  return {
    calls,
    async supportedModels() {
      calls.supportedModels++;
      if (models instanceof Error) throw models;
      return models;
    },
    async setModel(model) {
      calls.setModel.push(model);
      if (setModelError) throw setModelError;
    },
    async applyFlagSettings(settings) {
      calls.applyFlagSettings.push(settings);
      if (applyFlagError) throw applyFlagError;
    },
    async setMaxThinkingTokens(n) {
      calls.setMaxThinkingTokens.push(n);
      if (maxTokError) throw maxTokError;
    },
  };
}

test("no live session → no_session, SDK never touched", async () => {
  const { handler, events } = makeHarness({ query: null });
  await handler({ rid: "r1", model: "claude-sonnet-4-6" });
  assert.deepEqual(events, [
    { kind: "result", result: { rid: "r1", ok: false, code: "no_session", detail: "SDK session not running" } },
  ]);
});

test("Tier 2: a catalog miss on a well-formed id applies anyway, unverified, restamps the requested id", async () => {
  // The box already gated the id through its syntactic regex, so a catalog miss
  // is NOT unknown_model — the catalog under-enumerates. setModel fires, the
  // requested id is restamped (resolvedModel === the requested id), and the
  // result carries unverified:true. A bogus id surfaces on the next turn.
  const query = makeQuery({ models: [{ value: "claude-opus-4-8" }, { value: "sonnet", resolvedModel: "claude-sonnet-4-6" }] });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "r2", model: "claude-opus-4-6" });
  assert.deepEqual(query.calls.setModel, ["claude-opus-4-6"]);
  assert.deepEqual(events, [
    { kind: "applied", model: "claude-opus-4-6", resolvedModel: "claude-opus-4-6" },
    { kind: "result", result: { rid: "r2", ok: true, model: "claude-opus-4-6", unverified: true } },
  ]);
});

test("guard hit on value → setModel applied, mirror before announce", async () => {
  const query = makeQuery({ models: [{ value: "claude-opus-4-8" }] });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "r3", model: "claude-opus-4-8" });
  assert.deepEqual(query.calls.setModel, ["claude-opus-4-8"]);
  // Ordering is the contract: onApplied (env mirror + harness_info restamp)
  // strictly before the ok result the box persists on.
  assert.deepEqual(events, [
    { kind: "applied", model: "claude-opus-4-8", resolvedModel: "claude-opus-4-8" },
    { kind: "result", result: { rid: "r3", ok: true, model: "claude-opus-4-8" } },
  ]);
});

test("guard hit on an alias row's resolvedModel → applied, canonical id restamped", async () => {
  const query = makeQuery({ models: [{ value: "sonnet", resolvedModel: "claude-sonnet-4-6" }] });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "r4", model: "claude-sonnet-4-6" });
  assert.deepEqual(query.calls.setModel, ["claude-sonnet-4-6"]);
  assert.deepEqual(events[0], { kind: "applied", model: "claude-sonnet-4-6", resolvedModel: "claude-sonnet-4-6" });
});

test("alias set by value restamps the canonical resolved id", async () => {
  const query = makeQuery({ models: [{ value: "sonnet", resolvedModel: "claude-sonnet-4-6" }] });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "r5", model: "sonnet" });
  assert.deepEqual(query.calls.setModel, ["sonnet"]);
  assert.deepEqual(events, [
    { kind: "applied", model: "sonnet", resolvedModel: "claude-sonnet-4-6" },
    { kind: "result", result: { rid: "r5", ok: true, model: "sonnet" } },
  ]);
});

test("clear ('') skips the guard, applies setModel(undefined), resolved undefined", async () => {
  const query = makeQuery({ models: [{ value: "claude-opus-4-8" }] });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "r6", model: "" });
  assert.equal(query.calls.supportedModels, 0, "clear must not consult the model list");
  assert.deepEqual(query.calls.setModel, [undefined]);
  assert.deepEqual(events, [
    { kind: "applied", model: "", resolvedModel: undefined },
    { kind: "result", result: { rid: "r6", ok: true, model: "" } },
  ]);
});

test("setModel throw → apply_failed with first error line; no onApplied", async () => {
  const query = makeQuery({
    models: [{ value: "claude-opus-4-8" }],
    setModelError: new Error("control request rejected\nstack noise"),
  });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "r7", model: "claude-opus-4-8" });
  assert.deepEqual(events, [
    { kind: "result", result: { rid: "r7", ok: false, code: "apply_failed", detail: "control request rejected" } },
  ]);
});

test("supportedModels throw → apply_failed, session untouched", async () => {
  const query = makeQuery({ models: new Error("control transport closed") });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "r8", model: "claude-opus-4-8" });
  assert.equal(query.calls.setModel.length, 0);
  assert.deepEqual(events, [
    { kind: "result", result: { rid: "r8", ok: false, code: "apply_failed", detail: "control transport closed" } },
  ]);
});

test("malformed params are dropped without a result", async () => {
  const query = makeQuery({ models: [{ value: "claude-opus-4-8" }] });
  const { handler, events } = makeHarness({ query });
  await handler(undefined);
  await handler({});
  await handler({ rid: "", model: "claude-opus-4-8" });
  await handler({ rid: "r9" }); // model missing entirely
  await handler({ rid: 7, model: "claude-opus-4-8" });
  assert.deepEqual(events, []);
  assert.equal(query.calls.setModel.length, 0);
});

// ---- Robust model matching (Issue 1 root cause) -----------------------------

// The real supportedModels() shape observed live against a Max login: opus 4.8
// appears ONLY as resolvedModel "claude-opus-4-8[1m]" (context-tagged), while
// sonnet is the bare "claude-sonnet-5". Exact-match passed sonnet but the [1m]
// tag broke opus — the "isn't available" bug.
const LIVE_MODELS = [
  { value: "default", resolvedModel: "claude-opus-4-8[1m]" },
  { value: "opus[1m]", resolvedModel: "claude-opus-4-8[1m]" },
  { value: "claude-fable-5[1m]", resolvedModel: "claude-fable-5" },
  { value: "sonnet", resolvedModel: "claude-sonnet-5" },
  { value: "haiku", resolvedModel: "claude-haiku-4-5-20251001" },
];

test("matchModel: bare id matches a context-tagged resolvedModel (the opus 4.8 bug)", () => {
  // Restamps the catalog's fuller id so the box identity + client confirm agree.
  assert.equal(matchModel(LIVE_MODELS, "claude-opus-4-8"), "claude-opus-4-8[1m]");
});

test("matchModel: bare id matches a dated resolvedModel", () => {
  assert.equal(matchModel(LIVE_MODELS, "claude-haiku-4-5"), "claude-haiku-4-5-20251001");
});

test("matchModel: exact resolvedModel and value both match", () => {
  assert.equal(matchModel(LIVE_MODELS, "claude-sonnet-5"), "claude-sonnet-5");
  assert.equal(matchModel(LIVE_MODELS, "sonnet"), "claude-sonnet-5");
});

test("matchModel: a genuinely absent model is null (→ Tier 2 unverified apply, not a reject)", () => {
  assert.equal(matchModel(LIVE_MODELS, "claude-opus-4-6"), null);
});

test("handler: the opus-4-8 flip-back now applies instead of unknown_model", async () => {
  const query = makeQuery({ models: LIVE_MODELS });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "rop", model: "claude-opus-4-8" });
  assert.deepEqual(query.calls.setModel, ["claude-opus-4-8"]);
  assert.deepEqual(events, [
    { kind: "applied", model: "claude-opus-4-8", resolvedModel: "claude-opus-4-8[1m]" },
    { kind: "result", result: { rid: "rop", ok: true, model: "claude-opus-4-8" } },
  ]);
});

// ---- Effort hot apply (Issue 2) ---------------------------------------------

test("effort-only symbolic → applyFlagSettings({effortLevel}); model untouched", async () => {
  const query = makeQuery({ models: LIVE_MODELS });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "re1", effort: "xhigh" });
  assert.deepEqual(query.calls.applyFlagSettings, [{ effortLevel: "xhigh" }]);
  assert.equal(query.calls.setModel.length, 0, "effort-only must not touch the model");
  assert.equal(query.calls.supportedModels, 0, "effort-only must not consult the model list");
  assert.deepEqual(events, [
    { kind: "applied", effort: "xhigh" },
    { kind: "result", result: { rid: "re1", ok: true, effort: "xhigh" } },
  ]);
});

test("effort numeric → setMaxThinkingTokens(n)", async () => {
  const query = makeQuery({ models: LIVE_MODELS });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "re2", effort: "12000" });
  assert.deepEqual(query.calls.setMaxThinkingTokens, [12000]);
  assert.deepEqual(events[1], { kind: "result", result: { rid: "re2", ok: true, effort: "12000" } });
});

test("effort clear ('') drops both the effort level and the thinking budget", async () => {
  const query = makeQuery({ models: LIVE_MODELS });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "re3", effort: "" });
  assert.deepEqual(query.calls.applyFlagSettings, [{ effortLevel: null }]);
  assert.deepEqual(query.calls.setMaxThinkingTokens, [null]);
  assert.deepEqual(events, [
    { kind: "applied", effort: "" },
    { kind: "result", result: { rid: "re3", ok: true, effort: "" } },
  ]);
});

test("invalid effort → invalid_effort; session never touched", async () => {
  const query = makeQuery({ models: LIVE_MODELS });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "re4", effort: "turbo" });
  assert.equal(query.calls.applyFlagSettings.length, 0);
  assert.equal(query.calls.setMaxThinkingTokens.length, 0);
  assert.equal(events[0].kind, "result");
  assert.equal(events[0].result.ok, false);
  assert.equal(events[0].result.code, "invalid_effort");
});

test("combined model+effort → both applied, ONE restamp, ONE result echoing both", async () => {
  const query = makeQuery({ models: LIVE_MODELS });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "rc1", model: "claude-sonnet-5", effort: "high" });
  assert.deepEqual(query.calls.applyFlagSettings, [{ effortLevel: "high" }]);
  assert.deepEqual(query.calls.setModel, ["claude-sonnet-5"]);
  assert.deepEqual(events, [
    { kind: "applied", model: "claude-sonnet-5", resolvedModel: "claude-sonnet-5", effort: "high" },
    { kind: "result", result: { rid: "rc1", ok: true, model: "claude-sonnet-5", effort: "high" } },
  ]);
});

test("combined with a catalog-absent model → BOTH applied, unverified:true, one restamp echoing both", async () => {
  // Tier 2 on the combined path: the catalog-absent (but well-formed) model no
  // longer aborts the whole apply; effort lands too, and the single result
  // carries unverified:true.
  const query = makeQuery({ models: LIVE_MODELS });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "rc2", model: "claude-opus-4-6", effort: "high" });
  assert.deepEqual(query.calls.applyFlagSettings, [{ effortLevel: "high" }]);
  assert.deepEqual(query.calls.setModel, ["claude-opus-4-6"]);
  assert.deepEqual(events, [
    { kind: "applied", model: "claude-opus-4-6", resolvedModel: "claude-opus-4-6", effort: "high" },
    { kind: "result", result: { rid: "rc2", ok: true, model: "claude-opus-4-6", effort: "high", unverified: true } },
  ]);
});

test("firstLine caps at 200 chars and strips everything past the newline", () => {
  assert.equal(firstLine("plain"), "plain");
  assert.equal(firstLine("top\nrest"), "top");
  assert.equal(firstLine("x".repeat(300)).length, 200);
  assert.equal(firstLine(""), "");
});

// ---- Generation fencing (sol review #6) -------------------------------------
//
// An apply is several awaits long and the session under it can be replaced at
// any of them: runAgent ends one query and starts another, or the hotline child
// respawns. Capturing only the Query handle let an apply push a model onto a
// DEAD query and then report success — restamping harness_info and making the
// box persist to .env — for a session that never saw the change.

/** A session source that swaps the live query while supportedModels is in
 * flight, exactly as a stream end + restart does. */
function racingSession(first, second) {
  let current = { query: first, generation: 1 };
  return {
    get: () => current,
    swap: () => { current = { query: second, generation: 2 }; },
  };
}

test("generation: a query swapped during supportedModels does not get mutated", async () => {
  const dying = makeQuery({ models: [] });
  const live = makeQuery({ models: [] });
  const session = racingSession(dying, live);
  // The swap happens inside the await, which is where the real race lives.
  const original = dying.supportedModels.bind(dying);
  dying.supportedModels = async () => {
    const out = await original();
    session.swap();
    return out;
  };

  const { handler, events } = makeHarness({ getSession: session.get });
  await handler({ rid: "gen1", model: "claude-sonnet-4-6" });

  assert.deepEqual(dying.calls.setModel, [], "the dead query was mutated after it was replaced");
  assert.deepEqual(live.calls.setModel, [], "the apply leaked onto the replacement query");
  assert.deepEqual(
    events.filter((e) => e.kind === "applied"),
    [],
    "harness_info was restamped for a session that never applied the change",
  );
  assert.equal(events.length, 1);
  assert.equal(events[0].result.ok, false);
  assert.equal(events[0].result.code, "no_session", "a lost session must read as no_session, not success");
});

test("generation: a child respawn before the result also fails no_session", async () => {
  const query = makeQuery({ models: [{ value: "claude-sonnet-4-6" }] });
  let generation = 1;
  const { handler, events } = makeHarness({ getSession: () => ({ query, generation }) });
  // The child respawns after the controls land but before the restamp/result —
  // the window where the box would otherwise persist against a channel that no
  // longer carries this rid.
  const original = query.setModel.bind(query);
  query.setModel = async (m) => {
    await original(m);
    generation += 1;
  };

  await handler({ rid: "gen2", model: "claude-sonnet-4-6" });
  assert.deepEqual(
    events.filter((e) => e.kind === "applied"),
    [],
    "restamped harness_info across a child respawn",
  );
  assert.equal(events.at(-1).result.code, "no_session");
});

test("generation: an unchanged session applies normally (the fence is not a brake)", async () => {
  const query = makeQuery({ models: [{ value: "claude-sonnet-4-6" }] });
  const { handler, events } = makeHarness({ getSession: () => ({ query, generation: 7 }) });
  await handler({ rid: "gen3", model: "claude-sonnet-4-6" });
  assert.deepEqual(query.calls.setModel, ["claude-sonnet-4-6"]);
  assert.equal(events.at(-1).result.ok, true);
});

// ---- Effort is replacement state (sol review #7) ----------------------------
//
// The SDK has two independent effort controls and effortToSdkApply picks one
// per value. Setting only the chosen one leaves the previous mode's control
// still in force, so live behaviour diverges from a clean boot on the same
// .env — the exact drift the shared mapping exists to prevent.

test("effort numeric→symbolic clears the stale thinking budget", async () => {
  const query = makeQuery({ models: [] });
  const { handler } = makeHarness({ query });
  await handler({ rid: "e1", effort: "high" });
  assert.deepEqual(query.calls.applyFlagSettings, [{ effortLevel: "high" }]);
  assert.deepEqual(
    query.calls.setMaxThinkingTokens,
    [null],
    "a previously-set maxThinkingTokens budget stayed live under the new effortLevel",
  );
});

test("effort symbolic→numeric clears the stale effort level", async () => {
  const query = makeQuery({ models: [] });
  const { handler } = makeHarness({ query });
  await handler({ rid: "e2", effort: "20000" });
  assert.deepEqual(
    query.calls.applyFlagSettings,
    [{ effortLevel: null }],
    "a previously-set effortLevel stayed live under the new token budget",
  );
  assert.deepEqual(query.calls.setMaxThinkingTokens, [20000]);
});

test("effort: the clear path still drops both controls", async () => {
  const query = makeQuery({ models: [] });
  const { handler } = makeHarness({ query });
  await handler({ rid: "e3", effort: "" });
  assert.deepEqual(query.calls.applyFlagSettings, [{ effortLevel: null }]);
  assert.deepEqual(query.calls.setMaxThinkingTokens, [null]);
});

// ---- Delimiter-aware catalog matching (sol review #2) -----------------------
//
// Bare startsWith classified ANY prefix as a catalog hit, so junk was reported
// as a VERIFIED apply of whatever model happened to share its first letters —
// inverting the two-tier guard exactly where it matters and suppressing the
// caution that exists for unrecognized ids.

test("matchModel: a mid-token prefix like 'c' is NOT a catalog hit", () => {
  const models = [{ value: "claude-opus-4-8" }, { value: "claude-sonnet-4-6" }];
  // The review's case: bare startsWith made every one of these a VERIFIED hit
  // on claude-opus-4-8, so junk applied while claiming the catalog vouched for
  // it — and the Tier 2 caution, the one signal that something unusual is
  // happening, never rendered.
  assert.equal(matchModel(models, "c"), null);
  assert.equal(matchModel(models, "cl"), null);
  assert.equal(matchModel(models, "claude-o"), null);
  assert.equal(matchModel(models, "claude-opus-4-"), null, "a trailing delimiter is still mid-token");
});

test("matchModel: mid-token junk applies as Tier 2 unverified, not as a verified hit", async () => {
  const query = makeQuery({ models: [{ value: "claude-opus-4-8" }] });
  const { handler, events } = makeHarness({ query });
  await handler({ rid: "d1", model: "claude-o" });
  const result = events.at(-1).result;
  assert.equal(result.ok, true, "the box already syntax-gated it; Tier 2 applies anyway");
  assert.equal(
    result.unverified,
    true,
    "a mid-token id was reported as a VERIFIED catalog hit — the caution never renders",
  );
  assert.equal(events[0].resolvedModel, "claude-o", "junk must not be restamped as a real catalog id");
});

test("matchModel: a boundary-aligned prefix IS a hit — that is the alias tolerance, on purpose", () => {
  // Documented residual. "claude-opus-4" names a component boundary of
  // "claude-opus-4-8", and so does "claude". The delimiter rule cannot tell an
  // operator's shorthand from a too-short one, and it does not try to: the
  // catalog is advisory (the whole point of the two-tier guard), so a wrong
  // guess here costs an API error on the next turn, recoverable by switching
  // back. What the rule buys is that junk which names NO component — the "c"
  // case above — can no longer claim the catalog vouched for it.
  // A value-only row has no canonical to restamp, so the requested id is
  // returned (pre-existing semantics, unchanged); a row carrying resolvedModel
  // restamps that — the alias case this tolerance exists for.
  assert.equal(matchModel([{ value: "claude-opus-4-8" }], "claude-opus-4"), "claude-opus-4");
  assert.equal(
    matchModel([{ value: "opus", resolvedModel: "claude-opus-4-8-20260101" }], "claude-opus-4-8"),
    "claude-opus-4-8-20260101",
  );
});

test("idExtendsAtBoundary: the real alias cases still match", () => {
  assert.equal(idExtendsAtBoundary("claude-opus-4-8[1m]", "claude-opus-4-8"), true);
  assert.equal(idExtendsAtBoundary("claude-haiku-4-5-20251001", "claude-haiku-4-5"), true);
  assert.equal(idExtendsAtBoundary("claude-opus-4-8", "claude-opus-4"), true);
  assert.equal(idExtendsAtBoundary("claude-opus-4-8", "claude-opus-4-8"), false, "equal is exact, not extension");
  assert.equal(idExtendsAtBoundary("claude-opus-4-8", "c"), false);
  assert.equal(idExtendsAtBoundary("claude-opus-4-8", "claude-o"), false);
});
