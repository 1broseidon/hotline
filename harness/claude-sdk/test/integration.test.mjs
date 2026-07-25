/**
 * Hermetic integration: ChildManager over test/fake-hotline.mjs (a scripted
 * stdio MCP server). Asserts handshake → onReady tool snapshot → a scripted
 * channel notification reaches the inbound queue as an SDKUserMessage → a
 * proxy tools/call round-trips to the child. No Agent SDK session involved.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { ChildManager } from "../../core/dist/child.js";
import { mcpCallTool } from "../../core/dist/jsonrpc.js";
import { AsyncQueue } from "../../core/dist/queue.js";
import { toUserMessage } from "../dist/inbound.js";
import { buildHotlineProxy } from "../dist/proxy.js";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const FAKE = path.join(here, "../../core/test/fake-hotline.mjs");

test("child handshake, inbound notification, proxy round-trip", async () => {
  const queue = new AsyncQueue();
  let snapshot = [];
  let readyInit = null;

  let readyResolve;
  const ready = new Promise((r) => (readyResolve = r));

  const manager = new ChildManager({
    binary: process.execPath,
    args: [FAKE],
    env: { ...process.env, HOTLINE_FAKE_INJECT_MS: "30" },
    clientName: "hotline-claude-sdk",
    onNotification: (method, params) => {
      if (method !== "notifications/claude/channel") return;
      const msg = toUserMessage(params);
      if (msg) queue.push(msg);
    },
    onReady: (_client, init, tools) => {
      snapshot = tools;
      readyInit = init;
      readyResolve();
    },
    onDown: () => {},
    onFatal: (m) => {
      throw new Error(`unexpected fatal: ${m}`);
    },
  });
  manager.start();

  try {
    await Promise.race([
      ready,
      new Promise((_, rej) => setTimeout(() => rej(new Error("handshake timeout")), 5000)),
    ]);

    // Instructions and tools came through the real wire shape.
    assert.ok(readyInit.instructions.includes("hotline"));
    assert.deepEqual(
      snapshot.map((t) => t.name),
      ["reply", "react"],
    );

    // The scripted notification lands in the queue as a proper user message.
    const it = queue[Symbol.asyncIterator]();
    const first = await Promise.race([
      it.next(),
      new Promise((_, rej) => setTimeout(() => rej(new Error("notification never arrived")), 5000)),
    ]);
    assert.equal(first.done, false);
    assert.equal(first.value.type, "user");
    assert.match(first.value.message.content, /<channel source="telegram"/);
    assert.equal(first.value.parent_tool_use_id, null);

    // Direct child call round-trips.
    const direct = await mcpCallTool(manager.getClient(), "reply", { chat_id: "1", bubbles: ["yo"] });
    assert.equal(direct.isError, false);
    assert.equal(direct.content[0].text, "reply delivered");

    // And through the in-process MCP proxy, exactly as the SDK agent would.
    const proxy = buildHotlineProxy({ getClient: () => manager.getClient(), getTools: () => snapshot });
    const [ct, st] = InMemoryTransport.createLinkedPair();
    const mcpClient = new Client({ name: "test", version: "0.0.0" });
    await Promise.all([proxy.instance.server.connect(st), mcpClient.connect(ct)]);
    const listed = await mcpClient.listTools();
    assert.deepEqual(listed.tools, snapshot);
    const called = await mcpClient.callTool({ name: "reply", arguments: { chat_id: "1", bubbles: ["hi"] } });
    assert.equal(called.isError, false);
  } finally {
    manager.stop();
  }
});
