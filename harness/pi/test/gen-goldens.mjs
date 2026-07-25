#!/usr/bin/env node
/**
 * Generate test/goldens.json from the REAL hotline binary.
 *
 * The fake child (fake-hotline.mjs) and the unit tests (run-unit.mjs) both read
 * the frames this captures, so they can never drift from the Go source: the
 * initialize result (capabilities, serverInfo, the uncapped AgentInstructions)
 * and the full tools/list come straight off `hotline run`'s stdio, not a
 * hand-copied literal (review M3).
 *
 * What it does:
 *   1. `go build` the hotline binary from the repo (the module this test tree
 *      lives in), into a scratch dir.
 *   2. Spawn it as `hotline run` with HOTLINE_HARNESS=pi and a scratch
 *      HOTLINE_STATE_DIR (HOTLINE_YOLO unset — no token needed; the MCP
 *      handshake completes before any provider polling matters).
 *   3. Speak initialize + notifications/initialized + tools/list over stdio.
 *   4. Redact the scratch state-dir path (the only non-deterministic byte, it
 *      appears in the instructions' transcript-path line) to a stable
 *      placeholder so the checked-in file is reproducible.
 *   5. Write test/goldens.json.
 *
 * Regeneration: `npm run gen:goldens` (from harness/pi/) or `make goldens`
 * (from the repo root). CI runs it and fails if the tree is dirtied.
 */

import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const pkgRoot = path.resolve(here, ".."); // harness/pi
const repoRoot = path.resolve(pkgRoot, "..", ".."); // repo root (go module)
const goldensPath = path.join(here, "goldens.json");
const STATE_DIR_PLACEHOLDER = "<HOTLINE_STATE_DIR>";

function die(msg) {
  console.error(`gen-goldens: ${msg}`);
  process.exit(1);
}

function buildBinary(outDir) {
  const bin = path.join(outDir, "hotline");
  const res = spawnSync("go", ["build", "-o", bin, "./cmd/hotline"], {
    cwd: repoRoot,
    stdio: ["ignore", "inherit", "inherit"],
  });
  if (res.error) die(`go build failed to start (is Go installed?): ${res.error.message}`);
  if (res.status !== 0) die(`go build exited ${res.status}`);
  return bin;
}

async function captureFrames(bin, stateDir) {
  // review F3: run the binary under a WHITELISTED env, not the inherited host
  // env. HOTLINE_PROVIDERS is read from the process environment and changes the
  // captured tool schemas (>= 2 providers injects a `source` enum into every
  // tool), so a dev shell exporting `HOTLINE_PROVIDERS=telegram,signal` (or any
  // other HOTLINE_* knob) would churn the goldens machine-to-machine. Pin the
  // provider set to a single provider and pass only the vars the handshake needs.
  const env = {
    PATH: process.env.PATH ?? "",
    HOME: process.env.HOME ?? "",
    ...(process.env.TMPDIR ? { TMPDIR: process.env.TMPDIR } : {}),
    HOTLINE_HARNESS: "pi",
    HOTLINE_STATE_DIR: stateDir,
    HOTLINE_PROVIDERS: "telegram",
  };
  const child = spawn(bin, ["run"], { stdio: ["pipe", "pipe", "ignore"], env });

  let buf = "";
  const frames = [];
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    buf += chunk;
    let idx;
    while ((idx = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, idx);
      buf = buf.slice(idx + 1);
      if (line.trim()) frames.push(JSON.parse(line));
    }
  });

  const send = (obj) => child.stdin.write(JSON.stringify(obj) + "\n");
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  try {
    await sleep(300);
    send({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-06-18",
        capabilities: {},
        clientInfo: { name: "gen-goldens", version: "0" },
      },
    });
    await sleep(400);
    send({ jsonrpc: "2.0", method: "notifications/initialized" });
    send({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} });
    await sleep(600);
  } finally {
    child.stdin.end();
    child.kill("SIGTERM");
  }

  const init = frames.find((f) => f.id === 1)?.result;
  const tools = frames.find((f) => f.id === 2)?.result?.tools;
  if (!init) die("no initialize result captured");
  if (!Array.isArray(tools) || tools.length === 0) die("no tools/list captured");
  return { init, tools };
}

async function main() {
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "hotline-goldens-"));
  const stateDir = path.join(scratch, "state");
  fs.mkdirSync(stateDir, { recursive: true });
  try {
    const bin = buildBinary(scratch);
    let { init, tools } = await captureFrames(bin, stateDir);

    // Redact the only non-deterministic byte: the scratch state-dir path, which
    // rides inside the instructions' "Memory across restarts: …" line.
    const redact = (obj) =>
      JSON.parse(
        JSON.stringify(obj).split(stateDir).join(STATE_DIR_PLACEHOLDER),
      );
    init = redact(init);
    tools = redact(tools);

    const goldens = {
      _comment:
        "GENERATED by test/gen-goldens.mjs from the real hotline binary. Do not hand-edit; run `npm run gen:goldens` (or `make goldens`) to refresh. State-dir path redacted to " +
        STATE_DIR_PLACEHOLDER +
        ".",
      initialize: init,
      tools,
    };
    fs.writeFileSync(goldensPath, JSON.stringify(goldens, null, 2) + "\n");
    console.log(
      `gen-goldens: wrote ${path.relative(repoRoot, goldensPath)} (${tools.length} tools, instructions ${init.instructions.length} chars)`,
    );
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true });
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
