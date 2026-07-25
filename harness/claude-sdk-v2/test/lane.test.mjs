/**
 * M1/M2 integration: runAgent driving a real TurnLedger + fallback executor
 * against a scripted query, proving the full pipe — register → uuid echo →
 * text buffer → settle → send — and the M2 auth exit-code path.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import * as os from "node:os";
import * as path from "node:path";
import { AsyncQueue } from "../../core/dist/queue.js";
import { runAgent } from "../dist/agent.js";
import { TurnLedger } from "../dist/ledger.js";
import { createFallbackExecutor } from "../dist/fallback.js";
import { toInboundTurn } from "../dist/inbound.js";
import { classifyAuthMessage, classifyAuthThrow } from "../dist/authwatch.js";

const nolog = { info() {}, warn() {}, error() {} };
const sessionFile = () => path.join(os.tmpdir(), `hotline-lane-${Math.random().toString(36).slice(2)}.json`);

const within = async (ms, p, what) => {
  let t;
  const guard = new Promise((_, rej) => (t = setTimeout(() => rej(new Error(`${what} timed out`)), ms)));
  try {
    return await Promise.race([p, guard]);
  } finally {
    clearTimeout(t);
  }
};

function channel(source, chat, kind) {
  const k = kind ? ` kind="${kind}"` : "";
  return `<channel source="${source}" chat_id="${chat}"${k}>\nhi\n</channel>`;
}

/** Enqueue one inbound turn exactly as index.onNotification does. */
function enqueue(queue, ledger, content) {
  const turn = toInboundTurn({ content });
  ledger.register(turn.uuid, turn.env);
  queue.push(turn.msg);
  return turn.uuid;
}

/** A fake reply-recording client. */
function fakeClient() {
  const replies = [];
  return {
    replies,
    request(method, params) {
      if (params?.name === "reply") replies.push(params.arguments);
      return Promise.resolve({ isError: false, content: [] });
    },
  };
}

/** A scripted query: echo the first pulled turn's uuid, emit assistant text
 * (and optionally a reply tool_use), settle, end. */
function laneQuery({ withReply = false }) {
  return ({ prompt }) => {
    const iterator = prompt[Symbol.asyncIterator]();
    const run = async function* () {
      yield { type: "system", subtype: "init", session_id: "s1", model: "m" };
      const next = await iterator.next();
      if (!next.done) {
        const msg = next.value;
        yield { type: "user", uuid: msg.uuid, message: msg.message };
        const content = [{ type: "text", text: "the answer" }];
        if (withReply) content.push({ type: "tool_use", name: "mcp__hotline__reply", input: { chat_id: "1" } });
        yield { type: "assistant", message: { content } };
        yield { type: "result", subtype: "success", session_id: "s1" };
      }
    };
    const stream = run();
    return { [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator]() };
  };
}

function laneDeps(queue, ledger, fallbackExecutor, over = {}) {
  return {
    queue,
    proxy: { type: "sdk", name: "hotline", instance: {} },
    instructions: "",
    sessionFile: sessionFile(),
    savedSessionId: null,
    abortController: new AbortController(),
    isShuttingDown: () => false,
    onUserEcho: (uuid, isReplay) => ledger.onUserEcho(uuid, isReplay),
    onAssistantText: (texts) => ledger.onAssistantText(texts),
    onSettle: async (subtype) => {
      const plan = ledger.onResult(subtype, true);
      if (plan.fallbacks.length) await fallbackExecutor.run(plan.fallbacks);
    },
    onTeardown: () => ledger.revertOnTeardown(),
    ...over,
  };
}

test("operator turn ending in bare text → the executor sends one fallback", async () => {
  const queue = new AsyncQueue();
  const ledger = new TurnLedger();
  const client = fakeClient();
  const ex = createFallbackExecutor({ getClient: () => client, multiProvider: () => false, log: nolog, sleep: async () => {} });
  enqueue(queue, ledger, channel("telegram", "1"));

  const deps = laneDeps(queue, ledger, ex, { queryFn: laneQuery({ withReply: false }) });
  await within(5000, runAgent(deps), "runAgent");
  assert.equal(client.replies.length, 1);
  assert.equal(client.replies[0].chat_id, "1");
  assert.equal(client.replies[0].text, "the answer");
});

test("operator turn covered by a successful reply → no fallback", async () => {
  const queue = new AsyncQueue();
  const ledger = new TurnLedger();
  const client = fakeClient();
  const ex = createFallbackExecutor({ getClient: () => client, multiProvider: () => false, log: nolog, sleep: async () => {} });
  enqueue(queue, ledger, channel("telegram", "1"));

  const deps = laneDeps(queue, ledger, ex, {
    queryFn: laneQuery({ withReply: true }),
    // Simulate the PostToolUse reply-success hook (real SDK invokes it; a
    // scripted query does not).
    onMessage: (m) => {
      if (m.type === "assistant") {
        for (const b of m.message?.content ?? []) {
          if (b.type === "tool_use" && b.name === "mcp__hotline__reply") ledger.onReplySuccess(b.input?.source, b.input?.chat_id);
        }
      }
    },
  });
  await within(5000, runAgent(deps), "runAgent");
  // The reply-success hook covered the group, so the lane fires nothing.
  assert.equal(client.replies.length, 0);
});

test("a schedule turn ending in bare text → no fallback (no monologue leakage)", async () => {
  const queue = new AsyncQueue();
  const ledger = new TurnLedger();
  const client = fakeClient();
  const ex = createFallbackExecutor({ getClient: () => client, multiProvider: () => false, log: nolog, sleep: async () => {} });
  enqueue(queue, ledger, channel("telegram", "1", "schedule"));

  const deps = laneDeps(queue, ledger, ex, { queryFn: laneQuery({ withReply: false }) });
  await within(5000, runAgent(deps), "runAgent");
  assert.equal(client.replies.length, 0);
});

test("M2: a 401 throw before any result routes to onAuthFailure and returns its code", async () => {
  const queue = new AsyncQueue();
  let reset = 0;
  let failedWith = null;
  const deps = {
    queue,
    proxy: { type: "sdk", name: "hotline", instance: {} },
    instructions: "",
    sessionFile: sessionFile(),
    savedSessionId: null,
    abortController: new AbortController(),
    isShuttingDown: () => false,
    classifyAuthFailure: classifyAuthMessage,
    classifyAuthThrow: classifyAuthThrow,
    onAuthReset: () => reset++,
    onAuthFailure: async (e) => {
      failedWith = e;
      return 5;
    },
    queryFn: () => {
      const run = async function* () {
        throw new Error("HTTP 401 Unauthorized: invalid api key");
        // eslint-disable-next-line no-unreachable
        yield {};
      };
      const stream = run();
      return { [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator]() };
    },
  };
  const code = await within(5000, runAgent(deps), "runAgent");
  assert.equal(code, 5);
  assert.match(failedWith, /401/);
  assert.equal(reset, 0, "no init, so no reset");
});

test("M2: an auth_status error message drives onAuthFailure on stream end", async () => {
  const queue = new AsyncQueue();
  let failedWith = null;
  const deps = {
    queue,
    proxy: { type: "sdk", name: "hotline", instance: {} },
    instructions: "",
    sessionFile: sessionFile(),
    savedSessionId: null,
    abortController: new AbortController(),
    isShuttingDown: () => false,
    classifyAuthFailure: classifyAuthMessage,
    classifyAuthThrow: classifyAuthThrow,
    onAuthReset: () => {},
    onAuthFailure: async (e) => {
      failedWith = e;
      return 5;
    },
    queryFn: () => {
      const run = async function* () {
        yield { type: "auth_status", isAuthenticating: false, output: [], error: "invalid_api_key" };
        // stream ends without a result
      };
      const stream = run();
      return { [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator]() };
    },
  };
  const code = await within(5000, runAgent(deps), "runAgent");
  assert.equal(code, 5);
  assert.match(failedWith, /invalid_api_key/);
});

test("M2: a successful init resets auth state and a clean stream is exit 1 (not auth)", async () => {
  const queue = new AsyncQueue();
  let reset = 0;
  let authFailed = false;
  const deps = {
    queue,
    proxy: { type: "sdk", name: "hotline", instance: {} },
    instructions: "",
    sessionFile: sessionFile(),
    savedSessionId: null,
    abortController: new AbortController(),
    isShuttingDown: () => false,
    classifyAuthFailure: classifyAuthMessage,
    classifyAuthThrow: classifyAuthThrow,
    onAuthReset: () => reset++,
    onAuthFailure: async () => {
      authFailed = true;
      return 5;
    },
    queryFn: () => {
      const run = async function* () {
        yield { type: "system", subtype: "init", session_id: "s1", model: "m" };
        yield { type: "result", subtype: "success", session_id: "s1" };
      };
      const stream = run();
      return { [Symbol.asyncIterator]: () => stream[Symbol.asyncIterator]() };
    },
  };
  const code = await within(5000, runAgent(deps), "runAgent");
  assert.equal(reset, 1, "init reset auth state");
  assert.equal(authFailed, false);
  assert.equal(code, 1, "clean stream end is a normal respawn, not auth-fatal");
});
