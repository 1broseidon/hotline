/**
 * The M1 fallback executor (design §3.6): sends via reply on the live child,
 * one call per group, source only when multi-provider, one 5s retry on failure.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { createFallbackExecutor } from "../dist/fallback.js";

const nolog = { info() {}, warn() {}, error() {} };

/** A fake JsonRpcClient that records reply calls and answers per script. */
function fakeClient(script = { isError: false }) {
  const calls = [];
  return {
    calls,
    request(method, params) {
      calls.push({ method, params });
      if (script.throw) return Promise.reject(new Error("transport closed"));
      const s = typeof script.perCall === "function" ? script.perCall(calls.length) : script;
      return Promise.resolve({ isError: s.isError === true, content: [] });
    },
  };
}

test("fires one reply per group with chat_id + text; source omitted when single-provider", async () => {
  const client = fakeClient({ isError: false });
  const ex = createFallbackExecutor({
    getClient: () => client,
    multiProvider: () => false,
    log: nolog,
    sleep: async () => {},
  });
  await ex.run([{ source: "telegram", chat_id: "9", text: "hello", epoch: 0, outcome: "fallback" }]);
  assert.equal(client.calls.length, 1);
  assert.equal(client.calls[0].params.name, "reply");
  assert.equal(client.calls[0].params.arguments.chat_id, "9");
  assert.equal(client.calls[0].params.arguments.text, "hello");
  assert.equal(client.calls[0].params.arguments.source, undefined);
  assert.equal(ex.firedCount, 1);
});

test("includes source on a multi-provider box", async () => {
  const client = fakeClient({ isError: false });
  const ex = createFallbackExecutor({
    getClient: () => client,
    multiProvider: () => true,
    log: nolog,
    sleep: async () => {},
  });
  await ex.run([{ source: "discord", chat_id: "9", text: "hi", epoch: 0, outcome: "fallback" }]);
  assert.equal(client.calls[0].params.arguments.source, "discord");
});

test("an isError reply retries once after the delay", async () => {
  let slept = 0;
  const client = fakeClient({ perCall: (n) => ({ isError: n === 1 }) }); // first fails, second ok
  const ex = createFallbackExecutor({
    getClient: () => client,
    multiProvider: () => false,
    log: nolog,
    sleep: async () => {
      slept++;
    },
  });
  await ex.run([{ chat_id: "1", text: "x", epoch: 0, outcome: "fallback" }]);
  assert.equal(client.calls.length, 2);
  assert.equal(slept, 1);
});

test("a transport throw is caught and retried, never propagates", async () => {
  const client = fakeClient({ throw: true });
  const ex = createFallbackExecutor({
    getClient: () => client,
    multiProvider: () => false,
    log: nolog,
    sleep: async () => {},
  });
  await ex.run([{ chat_id: "1", text: "x", epoch: 0, outcome: "fallback" }]); // must not throw
  assert.equal(client.calls.length, 2); // one send + one retry
});

test("child down (null client) does not throw and does not count a send", async () => {
  const ex = createFallbackExecutor({
    getClient: () => null,
    multiProvider: () => false,
    log: nolog,
    sleep: async () => {},
  });
  await ex.run([{ chat_id: "1", text: "x", epoch: 0, outcome: "fallback" }]);
  assert.equal(ex.firedCount, 1); // fired (attempted), even though the send couldn't land
});

test("onFired fires once per lane, before the send", async () => {
  const client = fakeClient({ isError: false });
  const fired = [];
  const ex = createFallbackExecutor({
    getClient: () => client,
    multiProvider: () => false,
    log: nolog,
    sleep: async () => {},
    onFired: (a, bytes) => fired.push({ chat_id: a.chat_id, bytes }),
  });
  await ex.run([{ chat_id: "1", text: "abc", epoch: 0, outcome: "fallback" }]);
  assert.equal(fired.length, 1);
  assert.equal(fired[0].bytes, 3);
});
