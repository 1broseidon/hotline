/**
 * Hermetic integration: ChildManager over the shared fake-hotline stdio server.
 * Handshake → onReady tool snapshot → a scripted channel notification becomes a
 * uuid-stamped SDKUserMessage with its parsed envelope → the proxy tools/call
 * round-trips. No Agent SDK session involved.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { ChildManager } from "../../core/dist/child.js";
import { mcpCallTool } from "../../core/dist/jsonrpc.js";
import { AsyncQueue } from "../../core/dist/queue.js";
import { toInboundTurn } from "../dist/inbound.js";
import { buildHotlineProxy } from "../dist/proxy.js";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const FAKE = path.join(here, "../../core/test/fake-hotline.mjs");

test("child handshake, inbound turn with envelope + uuid, proxy round-trip", async () => {
  const queue = new AsyncQueue();
  let snapshot = [];
  let readyInit = null;
  let lastTurn = null;

  let readyResolve;
  const ready = new Promise((r) => (readyResolve = r));

  const manager = new ChildManager({
    binary: process.execPath,
    args: [FAKE],
    env: { ...process.env, HOTLINE_FAKE_INJECT_MS: "30" },
    clientName: "hotline-claude-sdk",
    onNotification: (method, params) => {
      if (method !== "notifications/claude/channel") return;
      const turn = toInboundTurn(params);
      if (turn) {
        lastTurn = turn;
        queue.push(turn.msg);
      }
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

    assert.ok(readyInit.instructions.includes("hotline"));
    assert.deepEqual(
      snapshot.map((t) => t.name),
      ["reply", "react"],
    );

    const it = queue[Symbol.asyncIterator]();
    const first = await Promise.race([
      it.next(),
      new Promise((_, rej) => setTimeout(() => rej(new Error("notification never arrived")), 5000)),
    ]);
    assert.equal(first.done, false);
    assert.equal(first.value.type, "user");
    assert.match(first.value.message.content, /<channel source="telegram"/);
    assert.equal(first.value.parent_tool_use_id, null);
    // The new inbound stamps a uuid and parses the envelope.
    assert.equal(typeof first.value.uuid, "string");
    assert.ok(first.value.uuid.length > 0);
    assert.equal(lastTurn.env.source, "telegram");
    assert.ok(lastTurn.env.chat_id);

    const direct = await mcpCallTool(manager.getClient(), "reply", { chat_id: "1", bubbles: ["yo"] });
    assert.equal(direct.isError, false);

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
