/**
 * runAgent's session lifecycle (sol review #5): the resume-retry path must not
 * eat inbound turns.
 *
 * A resume against a wiped/missing session throws — but only AFTER the SDK has
 * pulled turns from the streaming-input generator. The retry then built a
 * SECOND consumer over the same queue, so everything the first one pulled
 * (replayCatchup pushes an unbounded number at boot) was silently gone and the
 * dead generator's waiter was left holding a promise nobody would resolve. The
 * operator's messages simply never reached the agent, with nothing in the log
 * to say so.
 *
 * The query factory is injected so the real retry loop runs against a scripted
 * failing session; everything else here is production code.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import * as os from "node:os";
import * as path from "node:path";
import { AsyncQueue } from "../../core/dist/queue.js";
import { runAgent } from "../dist/agent.js";

/** Without leases the retry's iterator strands on the dead consumer's waiter
 * and runAgent never returns — so a regression here HANGS rather than failing.
 * Bound it so CI reports a failure instead of a timeout. */
const within = async (ms, promise, what) => {
  let timer;
  const guard = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what} did not settle within ${ms}ms — a stranded queue waiter`)), ms);
  });
  try {
    return await Promise.race([promise, guard]);
  } finally {
    clearTimeout(timer);
  }
};

const sessionFile = () => path.join(os.tmpdir(), `hotline-agent-test-${Math.random().toString(36).slice(2)}.json`);

const baseDeps = (over) => ({
  proxy: { type: "sdk", name: "hotline", instance: {} },
  instructions: "",
  sessionFile: sessionFile(),
  savedSessionId: null,
  abortController: new AbortController(),
  isShuttingDown: () => false,
  ...over,
});

/** A scripted query: pulls `pull` turns from the prompt generator, then either
 * throws (a failed resume) or emits `emit` messages and ends. */
function scriptedQuery({ pull = 0, fail = null, emit = [] }, record) {
  return ({ prompt }) => {
    const iterator = prompt[Symbol.asyncIterator]();
    const run = async function* () {
      for (let i = 0; i < pull; i++) {
        const next = await iterator.next();
        if (next.done) break;
        record.pulled.push(next.value);
      }
      if (fail) throw fail;
      for (const msg of emit) yield msg;
    };
    const stream = run();
    return {
      [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator](),
      async supportedModels() { return []; },
      async setModel() {},
      async applyFlagSettings() {},
      async setMaxThinkingTokens() {},
    };
  };
}

test("a failed resume hands its pulled turns back to the retry, in order", async () => {
  const queue = new AsyncQueue();
  for (const t of ["turn-1", "turn-2", "turn-3"]) queue.push(t);

  const record = { pulled: [] };
  let attempt = 0;
  const secondAttemptSaw = [];

  const deps = baseDeps({
    queue,
    savedSessionId: "wiped-session-id",
    queryFn: (args) => {
      attempt += 1;
      if (attempt === 1) {
        // The doomed resume: the SDK pulls the boot replay, THEN the resume
        // is rejected.
        return scriptedQuery(
          { pull: 2, fail: new Error("No conversation found with session ID: wiped-session-id") },
          record,
        )(args);
      }
      // The retry. Drain everything it can see, then end the stream cleanly.
      const iterator = args.prompt[Symbol.asyncIterator]();
      const run = async function* () {
        yield { type: "system", subtype: "init", session_id: "fresh", model: "claude-opus-4-8" };
        for (let i = 0; i < 3; i++) {
          const next = await Promise.race([
            iterator.next(),
            new Promise((r) => setTimeout(() => r({ done: true }), 50)),
          ]);
          if (next.done) break;
          secondAttemptSaw.push(next.value);
        }
      };
      const stream = run();
      return { [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator]() };
    },
  });

  await within(5000, runAgent(deps), "runAgent");

  assert.deepEqual(record.pulled, ["turn-1", "turn-2"], "the doomed attempt pulled the boot replay");
  assert.deepEqual(
    secondAttemptSaw,
    ["turn-1", "turn-2", "turn-3"],
    "turns pulled by the failed resume were lost, or replayed out of order",
  );
});

test("turns are NOT requeued once the session accepted them", async () => {
  // The other half of the contract: a session that produced messages consumed
  // its turns for real. Requeuing them on a later failure would duplicate the
  // operator's messages, which is its own kind of wrong.
  const queue = new AsyncQueue();
  queue.push("turn-1");

  const deps = baseDeps({
    queue,
    savedSessionId: "good-session",
    queryFn: ({ prompt }) => {
      const iterator = prompt[Symbol.asyncIterator]();
      const run = async function* () {
        yield { type: "system", subtype: "init", session_id: "good-session", model: "m" };
        await iterator.next(); // consumes turn-1, after the ack
      };
      const stream = run();
      return { [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator]() };
    },
  });

  await within(5000, runAgent(deps), "runAgent");
  assert.equal(queue.size, 0, "an accepted turn was returned to the queue and would be re-delivered");
});

test("a failed resume with nothing pulled still retries and still loses nothing", async () => {
  const queue = new AsyncQueue();
  queue.push("only-turn");
  let attempt = 0;
  const saw = [];

  const deps = baseDeps({
    queue,
    savedSessionId: "wiped",
    queryFn: ({ prompt }) => {
      attempt += 1;
      const iterator = prompt[Symbol.asyncIterator]();
      const first = attempt === 1;
      const run = async function* () {
        if (first) throw new Error("No conversation found with session ID: wiped");
        yield { type: "system", subtype: "init", session_id: "fresh", model: "m" };
        const next = await Promise.race([
          iterator.next(),
          new Promise((r) => setTimeout(() => r({ done: true }), 50)),
        ]);
        if (!next.done) saw.push(next.value);
      };
      const stream = run();
      return { [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator]() };
    },
  });

  await within(5000, runAgent(deps), "runAgent");
  assert.equal(attempt, 2, "the retry did not happen");
  assert.deepEqual(saw, ["only-turn"]);
});

test("onMessage receives every stream message", async () => {
  const queue = new AsyncQueue();
  const seen = [];
  const deps = baseDeps({
    queue,
    savedSessionId: null,
    onMessage: (msg) => seen.push(msg.type),
    queryFn: () => {
      const run = async function* () {
        yield { type: "system", subtype: "init", session_id: "s1", model: "m" };
        yield { type: "system", subtype: "task_started", task_id: "t1", description: "d", session_id: "s1" };
        yield { type: "result", subtype: "success", duration_ms: 1, total_cost_usd: 0, session_id: "s1" };
      };
      const stream = run();
      return { [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator]() };
    },
  });

  await within(5000, runAgent(deps), "runAgent");
  assert.deepEqual(seen, ["system", "system", "result"]);
});

test("a throwing onMessage never kills the loop", async () => {
  const queue = new AsyncQueue();
  const deps = baseDeps({
    queue,
    savedSessionId: null,
    onMessage: () => {
      throw new Error("onMessage boom");
    },
    queryFn: () => {
      const run = async function* () {
        yield { type: "system", subtype: "init", session_id: "s1", model: "m" };
        yield { type: "result", subtype: "success", duration_ms: 1, total_cost_usd: 0, session_id: "s1" };
      };
      const stream = run();
      return { [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator]() };
    },
  });

  const code = await within(5000, runAgent(deps), "runAgent");
  assert.equal(code, 1, "the stream ends (unexpectedly, per this scripted query) despite the throwing observer");
});

// ---- Mission control wiring (spec workstream 2) ----------------------------

test("mission hooks reach the query options and onResult polls per result", async () => {
  const queue = new AsyncQueue();
  const hooks = { PreCompact: [{ hooks: [async () => ({ continue: true })] }] };
  let seenOptions = null;
  const usageSeen = [];

  const deps = baseDeps({
    queue,
    savedSessionId: null,
    hooks,
    onResult: async (q) => usageSeen.push(await q.getContextUsage()),
    queryFn: ({ options }) => {
      seenOptions = options;
      const run = async function* () {
        yield { type: "system", subtype: "init", session_id: "s1", model: "m" };
        yield { type: "result", subtype: "success", session_id: "s1" };
      };
      const stream = run();
      return {
        [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator](),
        async getContextUsage() {
          return { totalTokens: 42, maxTokens: 100 };
        },
      };
    },
  });

  await within(5000, runAgent(deps), "runAgent");
  assert.equal(seenOptions.hooks, hooks, "Options.hooks carries the mission hooks verbatim");
  assert.deepEqual(usageSeen, [{ totalTokens: 42, maxTokens: 100 }], "onResult fired once, with the live query");
});

test("a loop-injected turn flows through the leased consumer", async () => {
  // The mission loop injects handoff / `/compact` turns via queue.push — the
  // same inbound path operator turns take. A turn pushed from onResult must
  // reach the query's streaming-input generator.
  const queue = new AsyncQueue();
  const injectedSeen = [];
  let pushed = false;

  const deps = baseDeps({
    queue,
    savedSessionId: null,
    onResult: async () => {
      if (pushed) return;
      pushed = true;
      queue.push({ type: "user", message: { role: "user", content: "/compact GO" }, parent_tool_use_id: null });
    },
    queryFn: ({ prompt }) => {
      const iterator = prompt[Symbol.asyncIterator]();
      const run = async function* () {
        yield { type: "system", subtype: "init", session_id: "s1", model: "m" };
        yield { type: "result", subtype: "success", session_id: "s1" }; // → onResult pushes
        const next = await Promise.race([
          iterator.next(),
          new Promise((r) => setTimeout(() => r({ done: true }), 50)),
        ]);
        if (!next.done) injectedSeen.push(next.value.message.content);
      };
      const stream = run();
      return {
        [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator](),
        async getContextUsage() {
          return { totalTokens: 1, maxTokens: 2 };
        },
      };
    },
  });

  await within(5000, runAgent(deps), "runAgent");
  assert.deepEqual(injectedSeen, ["/compact GO"], "the loop-injected turn reached the session generator");
});

test("compact_boundary drives onCompactBoundary with the trigger", async () => {
  const queue = new AsyncQueue();
  const boundaries = [];
  const deps = baseDeps({
    queue,
    savedSessionId: null,
    onCompactBoundary: (trigger) => boundaries.push(trigger),
    queryFn: () => {
      const run = async function* () {
        yield { type: "system", subtype: "init", session_id: "s1", model: "m" };
        yield {
          type: "system",
          subtype: "compact_boundary",
          session_id: "s1",
          compact_metadata: { trigger: "auto", pre_tokens: 123 },
        };
      };
      const stream = run();
      return { [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator]() };
    },
  });

  await within(5000, runAgent(deps), "runAgent");
  assert.deepEqual(boundaries, ["auto"], "the compact_boundary system message reset the cycle");
});
