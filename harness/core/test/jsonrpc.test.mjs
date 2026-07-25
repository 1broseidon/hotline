import { test } from "node:test";
import assert from "node:assert/strict";
import { JsonRpcClient, mcpCallTool, timeoutsFromEnv } from "../dist/jsonrpc.js";

function makeClient(notes = []) {
  const written = [];
  const client = new JsonRpcClient({
    out: { write: (chunk) => written.push(chunk) },
    onNotification: (method, params) => notes.push({ method, params }),
  });
  return { client, written, notes };
}

function lastMessage(written) {
  const lines = written.join("").split("\n").filter((l) => l.trim() !== "");
  return JSON.parse(lines[lines.length - 1]);
}

test("framing: split mid-line across chunks and \\r\\n endings", () => {
  const notes = [];
  const { client } = makeClient(notes);
  const msg = JSON.stringify({ jsonrpc: "2.0", method: "notifications/claude/channel", params: { content: "hi" } });
  client.feed(msg.slice(0, 10));
  assert.equal(notes.length, 0);
  client.feed(msg.slice(10) + "\r\n");
  assert.equal(notes.length, 1);
  assert.equal(notes[0].params.content, "hi");
});

test("request/response id matching", async () => {
  const { client } = makeClient();
  const p1 = client.request("tools/list");
  const p2 = client.request("tools/call");
  client.feed(JSON.stringify({ jsonrpc: "2.0", id: 2, result: { ok: 2 } }) + "\n");
  client.feed(JSON.stringify({ jsonrpc: "2.0", id: 1, result: { ok: 1 } }) + "\n");
  assert.deepEqual(await p1, { ok: 1 });
  assert.deepEqual(await p2, { ok: 2 });
});

test("ids start at 1", () => {
  const { client, written } = makeClient();
  void client.request("initialize").catch(() => {});
  assert.equal(lastMessage(written).id, 1);
});

test("timeout rejects and a late response is dropped", async () => {
  const { client } = makeClient();
  const p = client.request("tools/call", {}, 20);
  // The request timer is deliberately unref'd (jsonrpc.ts: a pending request
  // must never hold the process open), so it cannot keep the event loop alive
  // by itself. Without a ref'd timer here node drains the loop before the 20ms
  // timeout fires, `p` never settles, and the runner reports "Promise
  // resolution is still pending but the event loop has already resolved" —
  // taking the rest of the file down with it as cancelledByParent.
  const keepAlive = setTimeout(() => {}, 1000);
  try {
    await assert.rejects(p, /timed out after 20ms/);
  } finally {
    clearTimeout(keepAlive);
  }
  // Late response: dropped as unknown id, no throw.
  client.feed(JSON.stringify({ jsonrpc: "2.0", id: 1, result: {} }) + "\n");
});

test("error responses reject with method and code", async () => {
  const { client } = makeClient();
  const p = client.request("tools/call");
  client.feed(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32000, message: "boom" } }) + "\n");
  await assert.rejects(p, /tools\/call failed: boom \(code -32000\)/);
});

test("notification handler throw is contained", () => {
  const { client } = makeClient();
  const bad = new JsonRpcClient({
    out: { write: () => {} },
    onNotification: () => {
      throw new Error("handler bug");
    },
  });
  // Must not throw out of feed().
  bad.feed(JSON.stringify({ jsonrpc: "2.0", method: "notifications/x", params: {} }) + "\n");
  void client;
});

test("server->client request gets method-not-found reply", () => {
  const { client, written } = makeClient();
  client.feed(JSON.stringify({ jsonrpc: "2.0", id: 9, method: "sampling/createMessage", params: {} }) + "\n");
  const reply = lastMessage(written);
  assert.equal(reply.id, 9);
  assert.equal(reply.error.code, -32601);
});

test("close rejects all pending", async () => {
  const { client } = makeClient();
  const p = client.request("tools/call");
  client.close("child exited");
  await assert.rejects(p, /aborted: child exited/);
  await assert.rejects(client.request("x"), /client closed/);
});

test("unparseable and malformed lines are dropped silently", () => {
  const { client } = makeClient();
  client.feed("not json\n");
  client.feed(JSON.stringify({ jsonrpc: "2.0" }) + "\n");
});

// ---- Operator call-timeout override (sol review #8) -------------------------
//
// The core extraction moved mcpCallTool here with a DEFAULT timeout parameter.
// That is a quiet trap: a caller that forgets the argument still compiles, still
// runs, and silently ignores the operator's <PREFIX>_CALL_TIMEOUT_MS — which is
// exactly what happened to the pi harness's tool calls. These pin the contract
// so the default can never be mistaken for the resolved value.

test("timeoutsFromEnv resolves the operator's call timeout, per harness family", () => {
  const pi = timeoutsFromEnv("HOTLINE_PI", { HOTLINE_PI_CALL_TIMEOUT_MS: "180000" });
  assert.equal(pi.callTimeoutMs, 180000);
  const sdk = timeoutsFromEnv("HOTLINE_SDK", { HOTLINE_SDK_CALL_TIMEOUT_MS: "5000" });
  assert.equal(sdk.callTimeoutMs, 5000);
  // A family's var must not leak across harnesses.
  const other = timeoutsFromEnv("HOTLINE_SDK", { HOTLINE_PI_CALL_TIMEOUT_MS: "180000" });
  assert.notEqual(other.callTimeoutMs, 180000);
});

test("mcpCallTool honours an explicit timeout and falls back only when omitted", async () => {
  const seen = [];
  const client = {
    request(method, params, timeoutMs) {
      seen.push({ method, timeoutMs });
      return Promise.resolve({ content: [] });
    },
  };
  await mcpCallTool(client, "reply", {}, 180000);
  assert.deepEqual(seen.at(-1), { method: "tools/call", timeoutMs: 180000 });

  await mcpCallTool(client, "reply", {});
  assert.equal(
    seen.at(-1).timeoutMs,
    60000,
    "the documented default changed; harnesses omitting the argument silently inherit it",
  );
  assert.notEqual(
    seen.at(-1).timeoutMs,
    180000,
    "omitting the argument must NOT pick up an operator override — that is why callers have to pass it",
  );
});
