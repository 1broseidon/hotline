import { test } from "node:test";
import assert from "node:assert/strict";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import { buildHotlineProxy, CHANNEL_DOWN_MESSAGE } from "../dist/proxy.js";

const TOOLS = [
  {
    name: "reply",
    description: "Send a message",
    inputSchema: {
      type: "object",
      properties: { chat_id: { type: "string" }, bubbles: { type: "array", items: { type: "string" } } },
      required: ["chat_id", "bubbles"],
    },
  },
];

/** A stand-in for JsonRpcClient: only request() is used by mcpCallTool. */
function fakeChildClient(result) {
  const calls = [];
  return {
    calls,
    request: (method, params) => {
      calls.push({ method, params });
      return Promise.resolve(result);
    },
  };
}

async function connect(proxy) {
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  const client = new Client({ name: "test", version: "0.0.0" });
  await Promise.all([proxy.instance.server.connect(serverTransport), client.connect(clientTransport)]);
  return client;
}

test("proxy config shape matches the SDK's sdk-server contract", () => {
  const proxy = buildHotlineProxy({ getClient: () => null, getTools: () => [] });
  assert.equal(proxy.type, "sdk");
  assert.equal(proxy.name, "hotline");
  assert.ok(proxy.instance);
});

test("tools/list returns the snapshot verbatim (schema passthrough)", async () => {
  const proxy = buildHotlineProxy({ getClient: () => null, getTools: () => TOOLS });
  const client = await connect(proxy);
  const res = await client.listTools();
  assert.deepEqual(res.tools, TOOLS);
});

test("tools/call forwards name+arguments and returns the child result", async () => {
  const child = fakeChildClient({ content: [{ type: "text", text: "reply delivered" }], isError: false });
  const proxy = buildHotlineProxy({ getClient: () => child, getTools: () => TOOLS });
  const client = await connect(proxy);
  const res = await client.callTool({ name: "reply", arguments: { chat_id: "1", bubbles: ["hi"] } });
  assert.equal(res.isError, false);
  assert.equal(res.content[0].text, "reply delivered");
  assert.deepEqual(child.calls, [
    { method: "tools/call", params: { name: "reply", arguments: { chat_id: "1", bubbles: ["hi"] } } },
  ]);
});

test("child isError result passes through as isError, not a throw", async () => {
  const child = fakeChildClient({ content: [{ type: "text", text: "no bot token configured" }], isError: true });
  const proxy = buildHotlineProxy({ getClient: () => child, getTools: () => TOOLS });
  const client = await connect(proxy);
  const res = await client.callTool({ name: "reply", arguments: {} });
  assert.equal(res.isError, true);
  assert.equal(res.content[0].text, "no bot token configured");
});

test("client==null yields the channel-down isError result", async () => {
  const proxy = buildHotlineProxy({ getClient: () => null, getTools: () => TOOLS });
  const client = await connect(proxy);
  const res = await client.callTool({ name: "reply", arguments: {} });
  assert.equal(res.isError, true);
  assert.equal(res.content[0].text, CHANNEL_DOWN_MESSAGE);
});

test("missing arguments default to {}", async () => {
  const child = fakeChildClient({ content: [], isError: false });
  const proxy = buildHotlineProxy({ getClient: () => child, getTools: () => TOOLS });
  const client = await connect(proxy);
  await client.callTool({ name: "react" });
  assert.deepEqual(child.calls[0].params.arguments, {});
});

// ---- The real channel-down race (sol review #11) ----------------------------
//
// getClient() only catches a child that was ALREADY down. The race that
// actually happens is the child exiting DURING the call: the in-flight request
// rejects, and an uncaught rejection reaches the model as an MCP PROTOCOL
// error instead of the documented retryable isError. A protocol error reads as
// "this tool is broken"; the isError text says "retry shortly", which is true —
// the supervisor is already respawning.

/** A child client whose in-flight request rejects, the way a transport does
 * when the process it is talking to exits. */
function dyingChildClient(error) {
  return { request: () => Promise.reject(error) };
}

for (const [label, error] of [
  ["a closed transport", new Error("Connection closed")],
  ["a request timeout", Object.assign(new Error("Request timed out"), { code: -32001 })],
  ["an abort", new Error("The operation was aborted")],
]) {
  test(`child exiting mid-call (${label}) yields the retryable isError, not a protocol error`, async () => {
    const proxy = buildHotlineProxy({
      getClient: () => dyingChildClient(error),
      getTools: () => TOOLS,
    });
    const client = await connect(proxy);
    // callTool must RESOLVE with isError. If the rejection escapes the handler
    // the SDK turns it into a protocol error and this call throws instead.
    const res = await client.callTool({ name: "reply", arguments: { chat_id: "1" } });
    assert.equal(res.isError, true);
    assert.equal(res.content[0].text, CHANNEL_DOWN_MESSAGE);
  });
}

test("a tool's own isError result is still passed through untouched by the catch", async () => {
  // The catch must not swallow real tool failures into a generic channel-down
  // message: mcpCallTool RESOLVES for those, it does not throw.
  const proxy = buildHotlineProxy({
    getClient: () => fakeChildClient({ content: [{ type: "text", text: "chat_id not found" }], isError: true }),
    getTools: () => TOOLS,
  });
  const client = await connect(proxy);
  const res = await client.callTool({ name: "reply", arguments: { chat_id: "nope" } });
  assert.equal(res.isError, true);
  assert.equal(res.content[0].text, "chat_id not found");
});
