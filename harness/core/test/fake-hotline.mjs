#!/usr/bin/env node
/**
 * Fake `hotline run` child for hermetic tests (port of harness/pi/test/
 * fake-hotline.mjs). Speaks the same JSONL JSON-RPC frames the real Go child
 * emits over stdio. Unlike the pi original it inlines a minimal initialize/
 * tools fixture instead of generated goldens, so the suite runs offline with
 * no built hotline binary.
 *
 * Env knobs:
 *   HOTLINE_FAKE_OUT         write each tools/call it receives here (JSON), append.
 *   HOTLINE_FAKE_INJECT_MS   inject one channel notification this many ms after
 *                            notifications/initialized (0/unset = never).
 *   HOTLINE_FAKE_ENVELOPE    override the injected envelope content.
 *   HOTLINE_FAKE_REPLY_ERROR if "1", answer tools/call with isError=true.
 */

import * as fs from "node:fs";

const INIT_RESULT = {
  protocolVersion: "2025-06-18",
  capabilities: { tools: {} },
  serverInfo: { name: "hotline", version: "fake" },
  instructions: "You are reachable over hotline. Reply with the reply tool.",
};

const TOOLS = [
  {
    name: "reply",
    description: "Send a message to the person",
    inputSchema: {
      type: "object",
      properties: {
        chat_id: { type: "string" },
        bubbles: { type: "array", items: { type: "string" } },
      },
      required: ["chat_id", "bubbles"],
    },
  },
  {
    name: "react",
    description: "React with an emoji",
    inputSchema: { type: "object", properties: { emoji: { type: "string" } } },
  },
];

const DEFAULT_ENVELOPE =
  process.env.HOTLINE_FAKE_ENVELOPE ??
  '<channel source="telegram" chat_id="412587349" message_id="57" user="george">\nhey there\n</channel>';

const OUT = process.env.HOTLINE_FAKE_OUT;
const INJECT_MS = Number(process.env.HOTLINE_FAKE_INJECT_MS || 0);
const REPLY_ERROR = process.env.HOTLINE_FAKE_REPLY_ERROR === "1";

function send(msg) {
  process.stdout.write(JSON.stringify(msg) + "\n");
}

function log(msg) {
  process.stderr.write(`[fake-hotline] ${msg}\n`);
}

let buffer = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buffer += chunk;
  let idx;
  while ((idx = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, idx).replace(/\r$/, "");
    buffer = buffer.slice(idx + 1);
    if (line.trim() === "") continue;
    handle(line);
  }
});
process.stdin.on("end", () => process.exit(0));

function handle(line) {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch (e) {
    log(`bad line: ${e.message}`);
    return;
  }
  const { id, method, params } = msg;

  if (method === "initialize") {
    send({
      jsonrpc: "2.0",
      id,
      result: {
        ...INIT_RESULT,
        protocolVersion: params?.protocolVersion ?? INIT_RESULT.protocolVersion,
      },
    });
    return;
  }

  if (method === "notifications/initialized") {
    if (INJECT_MS > 0) {
      setTimeout(() => {
        log("injecting channel notification");
        send({
          jsonrpc: "2.0",
          method: "notifications/claude/channel",
          params: { content: DEFAULT_ENVELOPE, meta: null },
        });
      }, INJECT_MS);
    }
    return;
  }

  if (method === "tools/list") {
    send({ jsonrpc: "2.0", id, result: { tools: TOOLS } });
    return;
  }

  if (method === "tools/call") {
    const name = params?.name;
    const args = params?.arguments;
    if (OUT) {
      try {
        fs.appendFileSync(OUT, JSON.stringify({ name, args }) + "\n");
      } catch (e) {
        log(`OUT write failed: ${e.message}`);
      }
    }
    if (REPLY_ERROR) {
      send({
        jsonrpc: "2.0",
        id,
        result: { content: [{ type: "text", text: "no bot token configured" }], isError: true },
      });
    } else {
      send({
        jsonrpc: "2.0",
        id,
        result: { content: [{ type: "text", text: `${name} delivered` }], isError: false },
      });
    }
    return;
  }

  if (id !== undefined && id !== null) {
    send({ jsonrpc: "2.0", id, error: { code: -32601, message: `method not found: ${method}` } });
  }
}

log(`ready (inject=${INJECT_MS}ms, out=${OUT ?? "none"})`);
