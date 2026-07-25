#!/usr/bin/env node
/**
 * Fake `hotline run` child for testing the hotline-pi extension end-to-end.
 *
 * It speaks EXACTLY the JSON-RPC frames the real Go child emits over stdio, so
 * the extension's hand-rolled JSONL client is exercised against the true wire
 * shape without a bot token or a live Telegram:
 *
 *   - newline-delimited JSON (one message per line, single "\n"), same as the
 *     Go SDK's StdioTransport.
 *   - initialize result: capabilities, serverInfo, and the uncapped
 *     AgentInstructions block — all read from test/goldens.json, which is
 *     GENERATED from the real `hotline run` binary (gen-goldens.mjs), so this
 *     fake can never silently drift from the Go source (review M3).
 *   - tools/list: the channel tools with the exact InputSchema and descriptions
 *     the real binary emits, again straight from goldens.json.
 *   - notifications/claude/channel: the pre-rendered <channel …> envelope in
 *     params.content with meta=null — the exact golden from run_pi_test.go
 *     (TestPiSinkRendersChannelEnvelope). This one stays pinned here because it
 *     is a rendered inbound turn (needs a live message to capture), and it is
 *     already byte-pinned on the Go side in that test.
 *
 * Env knobs:
 *   HOTLINE_FAKE_OUT       write each tools/call it receives here (JSON), append.
 *   HOTLINE_FAKE_INJECT_MS inject one channel notification this many ms after
 *                          notifications/initialized (0/unset = never).
 *   HOTLINE_FAKE_ENVELOPE  override the injected envelope content.
 *   HOTLINE_FAKE_REPLY_ERROR if "1", answer tools/call with isError=true.
 */

import * as fs from "node:fs";
import { fileURLToPath } from "node:url";
import * as path from "node:path";

// Frames captured from the real hotline binary (see gen-goldens.mjs). If this
// is missing, the goldens were never generated — fail loudly rather than test
// against nothing.
const here = path.dirname(fileURLToPath(import.meta.url));
const goldensPath = path.join(here, "goldens.json");
if (!fs.existsSync(goldensPath)) {
  process.stderr.write(
    `[fake-hotline] test/goldens.json missing — run 'npm run gen:goldens' (or 'make goldens') first\n`,
  );
  process.exit(2);
}
const GOLDENS = JSON.parse(fs.readFileSync(goldensPath, "utf8"));
const INIT_RESULT = GOLDENS.initialize;
const TOOLS = GOLDENS.tools;

// Golden envelope from run_pi_test.go TestPiSinkRendersChannelEnvelope.
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
// Keep the process alive on stdin EOF handling like the real child (exit on EOF).
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
    // Serve the exact capabilities/serverInfo/instructions the real binary
    // emits (from goldens.json), overriding only protocolVersion to echo the
    // client's, matching how the real handshake negotiates it.
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

  // Unknown request: answer so the client never hangs.
  if (id !== undefined && id !== null) {
    send({ jsonrpc: "2.0", id, error: { code: -32601, message: `method not found: ${method}` } });
  }
}

log(`ready (inject=${INJECT_MS}ms, out=${OUT ?? "none"})`);
