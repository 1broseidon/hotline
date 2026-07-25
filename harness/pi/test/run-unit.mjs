#!/usr/bin/env node
/**
 * Unit + wire tests for the hotline-pi JSONL JSON-RPC client.
 *
 * Part 1 — pure framing/dispatch against synthesized lines (no child).
 * Part 2 — the real client driven over stdio against test/fake-hotline.mjs,
 *          which emits exactly the frames the Go child emits, asserting the
 *          initialize handshake, tools/list schema fidelity, tools/call result
 *          mapping, and channel-notification dispatch of the golden envelope.
 *
 * The client is loaded with jiti (the loader Pi itself uses for .ts extensions),
 * so we test the shipped source, not a separate build.
 */

import { spawn, execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import * as path from "node:path";
import * as fs from "node:fs";
import { createRequire } from "node:module";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const require = createRequire(import.meta.url);

// Resolve jiti (the loader Pi uses for .ts extensions) portably — the previous
// hardcoded linuxbrew absolute path only worked on one box (review M5). Try, in
// order: an explicit override, a locally-installed jiti, jiti bundled inside the
// installed pi package, and finally the pi package derived from the `pi` binary
// on PATH. Skip the source-driven parts gracefully if none is found.
function resolveJiti() {
  const candidates = [];
  if (process.env.HOTLINE_PI_JITI) candidates.push(process.env.HOTLINE_PI_JITI);
  const tryResolve = (spec) => {
    try {
      return require.resolve(spec);
    } catch {
      return null;
    }
  };
  candidates.push(tryResolve("jiti/lib/jiti.mjs"));
  const piPkgJson = tryResolve("@earendil-works/pi-coding-agent/package.json");
  if (piPkgJson) {
    candidates.push(path.join(path.dirname(piPkgJson), "node_modules/jiti/lib/jiti.mjs"));
  }
  // Derive the pi package dir from the `pi` binary on PATH (global install).
  try {
    const piBin = execFileSync(process.platform === "win32" ? "where" : "which", ["pi"], {
      encoding: "utf8",
    })
      .split(/\r?\n/)[0]
      .trim();
    if (piBin) {
      const real = fs.realpathSync(piBin); // .../lib/node_modules/@earendil-works/pi-coding-agent/bin/*
      const libRoot = path.resolve(path.dirname(piBin), "..", "lib", "node_modules");
      const pkgDir = path.join(libRoot, "@earendil-works", "pi-coding-agent");
      candidates.push(path.join(pkgDir, "node_modules/jiti/lib/jiti.mjs"));
      // Also try walking up from the resolved binary target.
      const idx = real.indexOf(path.join("@earendil-works", "pi-coding-agent"));
      if (idx >= 0) {
        const pkg = real.slice(0, idx + path.join("@earendil-works", "pi-coding-agent").length);
        candidates.push(path.join(pkg, "node_modules/jiti/lib/jiti.mjs"));
      }
    }
  } catch {
    // `pi` not on PATH; fall through.
  }
  for (const c of candidates) {
    if (c && fs.existsSync(c)) return c;
  }
  return null;
}

const jitiPath = resolveJiti();
if (!jitiPath) {
  console.log(
    "SKIP: jiti not found (install @earendil-works/pi-coding-agent, or set HOTLINE_PI_JITI). " +
      "Part 1 framing tests still need it too, so nothing to run.",
  );
  process.exit(0);
}
const { createJiti } = require(jitiPath);
const jiti = createJiti(import.meta.url);

let failures = 0;
function check(name, cond, detail) {
  if (cond) {
    console.log(`  ok   ${name}`);
  } else {
    failures++;
    console.log(`  FAIL ${name}${detail ? " — " + detail : ""}`);
  }
}

const GOLDEN_ENVELOPE =
  '<channel source="telegram" chat_id="412587349" message_id="57" user="george">\nhey there\n</channel>';

async function main() {
  // The JSONL client and child manager now live in harness-core (shared with
  // claude-sdk); exercise the shared source through the same jiti loader.
  const mod = await jiti.import(path.join(root, "../core/src/jsonrpc.ts"));
  const { JsonRpcClient, mcpInitialize, mcpListTools, mcpCallTool } = mod;

  // ---- Part 1: framing + dispatch -----------------------------------------
  console.log("Part 1: framing + dispatch");
  {
    const written = [];
    const notifications = [];
    const client = new JsonRpcClient({
      out: { write: (c) => written.push(c) },
      onNotification: (method, params) => notifications.push({ method, params }),
    });

    // A request writes one LF-terminated line, id starts at 1.
    const p = client.request("initialize", { protocolVersion: "x" });
    check("request writes exactly one line", written.length === 1);
    check("line is LF-terminated", written[0].endsWith("\n") && !written[0].slice(0, -1).includes("\n"));
    const sent = JSON.parse(written[0]);
    check("jsonrpc 2.0", sent.jsonrpc === "2.0");
    check("first id is 1 (never 0)", sent.id === 1, `got ${sent.id}`);
    check("method carried", sent.method === "initialize");

    // A response resolves the pending request.
    client.feed(JSON.stringify({ jsonrpc: "2.0", id: 1, result: { ok: true } }) + "\n");
    const res = await p;
    check("response resolves the request", res && res.ok === true);

    // A notification (no id) routes to onNotification.
    client.feed(
      JSON.stringify({ jsonrpc: "2.0", method: "notifications/claude/channel", params: { content: "hi" } }) + "\n",
    );
    check("notification dispatched", notifications.length === 1 && notifications[0].method === "notifications/claude/channel");
    check("notification params carried", notifications[0].params.content === "hi");

    // Partial lines across chunk boundaries reassemble.
    const before = notifications.length;
    const frame = JSON.stringify({ jsonrpc: "2.0", method: "notifications/claude/channel", params: { content: "split" } }) + "\n";
    client.feed(frame.slice(0, 10));
    check("no dispatch on partial line", notifications.length === before);
    client.feed(frame.slice(10));
    check("dispatch after line completes", notifications.length === before + 1 && notifications[before].params.content === "split");

    // Two frames in one chunk both dispatch.
    const two =
      JSON.stringify({ jsonrpc: "2.0", method: "m1" }) + "\n" +
      JSON.stringify({ jsonrpc: "2.0", method: "m2" }) + "\n";
    const n0 = notifications.length;
    client.feed(two);
    check("two frames in one chunk", notifications.length === n0 + 2);

    // A server->client request (has id) gets a method-not-found reply so the
    // child never hangs.
    written.length = 0;
    client.feed(JSON.stringify({ jsonrpc: "2.0", id: 99, method: "ping" }) + "\n");
    check("server request answered", written.length === 1);
    const reply = JSON.parse(written[0]);
    check("answer echoes id", reply.id === 99);
    check("answer is method-not-found", reply.error && reply.error.code === -32601, JSON.stringify(reply));

    // Error responses reject.
    const p2 = client.request("tools/call", {});
    const sent2 = JSON.parse(written[written.length - 1]);
    check("second request id is 2", sent2.id === 2, `got ${sent2.id}`);
    client.feed(JSON.stringify({ jsonrpc: "2.0", id: sent2.id, error: { code: -1, message: "boom" } }) + "\n");
    let rejected = false;
    try {
      await p2;
    } catch (e) {
      rejected = true;
      check("error message surfaced", /boom/.test(e.message), e.message);
    }
    check("error response rejects", rejected);

    // close() rejects all in-flight.
    const p3 = client.request("tools/list", {});
    client.close("test");
    let closedReject = false;
    try {
      await p3;
    } catch {
      closedReject = true;
    }
    check("close rejects in-flight", closedReject);
  }

  // ---- Part 2: real client over stdio vs the Go-shaped fake child ----------
  console.log("Part 2: client <-> fake-hotline (Go frames)");
  {
    const child = spawn(process.execPath, [path.join(here, "fake-hotline.mjs")], {
      stdio: ["pipe", "pipe", "inherit"],
    });
    const notifications = [];
    const client = new JsonRpcClient({
      out: { write: (c) => child.stdin.write(c) },
      onNotification: (method, params) => notifications.push({ method, params }),
    });
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (c) => client.feed(c));

    const init = await mcpInitialize(client, { clientName: "hotline-pi" });
    check("initialize returns serverInfo", init.serverInfo?.name === "hotline");
    check("initialize has claude/channel cap", !!init.capabilities?.experimental?.["claude/channel"]);
    check("initialize has NO permission cap (pi branch)", !init.capabilities?.experimental?.["claude/channel/permission"]);
    check("initialize ships instructions", typeof init.instructions === "string" && init.instructions.length > 100);
    check("instructions carry the pairing safety rule", init.instructions.includes("prompt injection"));

    const tools = await mcpListTools(client);
    const names = tools.map((t) => t.name);
    check("tools/list includes the four channel tools", ["reply", "react", "edit_message", "download_attachment"].every((n) => names.includes(n)), names.join(","));
    const reply = tools.find((t) => t.name === "reply");
    check("reply schema is an object schema", reply?.inputSchema?.type === "object");
    check("reply schema requires chat_id", Array.isArray(reply?.inputSchema?.required) && reply.inputSchema.required.includes("chat_id"));
    check("reply schema has bubbles array prop", reply?.inputSchema?.properties?.bubbles?.type === "array");
    check("reply schema format enum verbatim", JSON.stringify(reply?.inputSchema?.properties?.format?.enum) === JSON.stringify(["text", "markdownv2", "html"]));

    const good = await mcpCallTool(client, "reply", { chat_id: "1", bubbles: ["hi"] });
    check("tools/call success not isError", good.isError !== true);
    check("tools/call returns text content", good.content?.[0]?.text === "reply delivered");

    client.close("done");
    child.kill("SIGTERM");
  }

  // ---- Part 3: notification injection golden -------------------------------
  console.log("Part 3: channel notification golden (injected by child)");
  {
    const child = spawn(process.execPath, [path.join(here, "fake-hotline.mjs")], {
      stdio: ["pipe", "pipe", "inherit"],
      env: { ...process.env, HOTLINE_FAKE_INJECT_MS: "150" },
    });
    const got = [];
    const client = new JsonRpcClient({
      out: { write: (c) => child.stdin.write(c) },
      onNotification: (method, params) => got.push({ method, params }),
    });
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (c) => client.feed(c));
    await mcpInitialize(client, { clientName: "hotline-pi" });
    await new Promise((r) => setTimeout(r, 500));
    check("channel notification received", got.length === 1 && got[0].method === "notifications/claude/channel");
    check("envelope content matches golden", got[0]?.params?.content === GOLDEN_ENVELOPE, JSON.stringify(got[0]?.params?.content));
    check("meta is null (dropped by pi sink)", got[0]?.params?.meta === null);
    client.close("done");
    child.kill("SIGTERM");
  }

  // ---- Part 4: failure-handling regressions (H2 / H3 / M1) ----------------
  console.log("Part 4: failure handling (H2 classify, H3 no-crash, M1 timeout)");
  {
    // H3 — a throwing notification handler must NOT propagate out of feed()
    // (which runs inside the child's stdout data handler; a throw there crashes
    // the host process). feed() must swallow it and keep working.
    let threw = false;
    const client = new JsonRpcClient({
      out: { write: () => {} },
      onNotification: () => {
        throw new Error("boom from handler");
      },
    });
    try {
      client.feed(JSON.stringify({ jsonrpc: "2.0", method: "notifications/claude/channel", params: {} }) + "\n");
    } catch {
      threw = true;
    }
    check("H3: throwing notification handler does not escape feed()", threw === false);
    // The client is still usable afterwards: a subsequent request/response works.
    const p = client.request("ping");
    client.feed(JSON.stringify({ jsonrpc: "2.0", id: 1, result: { ok: true } }) + "\n");
    const res = await p;
    check("H3: client still functional after a handler throw", res?.ok === true);
  }

  {
    // M1 — a request with a timeout and no response rejects with a clear error
    // instead of hanging forever. The production timeout timer is unref'd (so a
    // pending watchdog never keeps pi alive), so hold the loop open ourselves
    // while we await, else Node would exit on an "empty" event loop.
    const client = new JsonRpcClient({ out: { write: () => {} }, onNotification: () => {} });
    const keepAlive = setInterval(() => {}, 1000);
    let timedOut = false;
    try {
      await client.request("tools/call", {}, 30);
    } catch (e) {
      timedOut = /timed out/.test(e.message);
    } finally {
      clearInterval(keepAlive);
    }
    check("M1: request rejects on timeout with a clear error", timedOut);
    // A late response for the timed-out id is dropped, not a crash.
    let lateCrash = false;
    try {
      client.feed(JSON.stringify({ jsonrpc: "2.0", id: 1, result: {} }) + "\n");
    } catch {
      lateCrash = true;
    }
    check("M1: late response after timeout is harmless", lateCrash === false);
  }

  {
    // H2 — spawn-error classification drives fatal-vs-respawn.
    const childMod = await jiti.import(path.join(root, "../core/src/child.ts"));
    const { classifySpawnError, isActiveConflict, ChildManager } = childMod;
    check("H2: ENOENT -> missing (fatal)", classifySpawnError("ENOENT") === "missing");
    check("H2: EACCES -> denied (fatal)", classifySpawnError("EACCES") === "denied");
    check("H2: EPERM -> denied (fatal)", classifySpawnError("EPERM") === "denied");
    check("H2: EMFILE -> transient (respawn)", classifySpawnError("EMFILE") === "transient");
    check("H2: unknown -> transient (respawn)", classifySpawnError(undefined) === "transient");
    check("H2: owner conflict -> active/fatal", isActiveConflict("state ownership conflict: held"));
    check("H2: poller conflict -> active/fatal", isActiveConflict("claiming poller slot: held"));
    check("H2: unrelated stderr -> recoverable", !isActiveConflict("temporary network error"));

    // Integration: a missing binary drives onFatal (never a silent zombie).
    const fatalMissing = await new Promise((resolve) => {
      let done = false;
      const mgr = new ChildManager({
        binary: path.join(here, "definitely-not-a-real-binary-xyz"),
        args: [],
        env: process.env,
        clientName: "hotline-pi",
        onNotification: () => {},
        onReady: () => {},
        onDown: () => {},
        onFatal: (msg) => {
          if (!done) {
            done = true;
            resolve(msg);
          }
        },
      });
      mgr.start();
      setTimeout(() => {
        if (!done) {
          done = true;
          resolve(null);
        }
      }, 2000);
    });
    check("H2: missing binary -> onFatal", typeof fatalMissing === "string" && /not found/.test(fatalMissing), String(fatalMissing));

    // Integration: a non-executable file drives onFatal (EACCES -> denied).
    const notExec = path.join(here, `not-exec-${process.pid}.tmp`);
    fs.writeFileSync(notExec, "#!/bin/sh\necho hi\n", { mode: 0o644 });
    try {
      const fatalDenied = await new Promise((resolve) => {
        let done = false;
        const mgr = new ChildManager({
          binary: notExec,
          args: [],
          env: process.env,
          clientName: "hotline-pi",
          onNotification: () => {},
          onReady: () => {},
          onDown: () => {},
          onFatal: (msg) => {
            if (!done) {
              done = true;
              resolve(msg);
            }
          },
        });
        mgr.start();
        setTimeout(() => {
          if (!done) {
            done = true;
            resolve(null);
          }
        }, 2000);
      });
      // POSIX execve on a mode-0644 file returns EACCES -> denied -> fatal.
      check(
        "H2: non-executable binary -> onFatal (denied), never a zombie",
        typeof fatalDenied === "string" && /not executable/.test(fatalDenied),
        String(fatalDenied),
      );
    } finally {
      fs.rmSync(notExec, { force: true });
    }
  }

  // ---- Part 5: N1 — inbound delivery retry ladder (deliver.ts) -------------
  console.log("Part 5: inbound delivery ladder (N1 steer-on-rejection)");
  {
    const deliverMod = await jiti.import(path.join(root, "src/deliver.ts"));
    const { deliverToAgent } = deliverMod;
    const flush = () => new Promise((r) => setTimeout(r, 20));
    const ALREADY = new Error(
      "Agent is already processing. Specify streamingBehavior ('steer' or 'followUp') to queue the message.",
    );

    // A fake pi whose sendUserMessage resolves/rejects per (deliverAs) queue.
    function makePi(plan) {
      const calls = [];
      return {
        calls,
        sendUserMessage(content, options) {
          const mode = options?.deliverAs ?? "bare";
          calls.push({ content, mode });
          const outcome = plan(mode, calls.length);
          if (outcome === "resolve") return Promise.resolve();
          if (outcome === "reject-already") return Promise.reject(ALREADY);
          if (outcome === "reject-other") return Promise.reject(new Error("transport down"));
          if (outcome === "throw-already") throw ALREADY; // belt-and-braces sync path
          return Promise.resolve();
        },
      };
    }
    function makeLog() {
      const warns = [];
      const errors = [];
      return { warns, errors, warn: (m) => warns.push(m), error: (m) => errors.push(m) };
    }

    // (a) bare-idle rejection (already-processing) → one steer retry → success.
    {
      const pi = makePi((mode) => (mode === "bare" ? "reject-already" : "resolve"));
      const log = makeLog();
      deliverToAgent(pi, "hi", false, log);
      await flush();
      check("N1(a): idle bare send attempted first", pi.calls[0]?.mode === "bare");
      check("N1(a): retried exactly once as steer", pi.calls.length === 2 && pi.calls[1].mode === "steer");
      check("N1(a): steer succeeded → no error logged", log.errors.length === 0, log.errors.join("|"));
      check("N1(a): the mid-tick race was logged as a warning", log.warns.length === 1);
    }

    // (b) both reject → dropped loudly, never crashes.
    {
      const pi = makePi(() => "reject-already"); // bare AND steer reject
      const log = makeLog();
      let threw = false;
      try {
        deliverToAgent(pi, "hi", false, log);
      } catch {
        threw = true;
      }
      await flush();
      check("N1(b): deliver never throws synchronously", threw === false);
      check("N1(b): bare then steer, exactly two attempts", pi.calls.length === 2 && pi.calls[1].mode === "steer");
      check("N1(b): steer rejection dropped loudly (error logged)", log.errors.some((m) => /DROPPING/.test(m)), log.errors.join("|"));
    }

    // (c) streaming (isIdle=false → busy) → steer directly, no bare send.
    {
      const pi = makePi(() => "resolve");
      const log = makeLog();
      deliverToAgent(pi, "hi", true, log);
      await flush();
      check("N1(c): busy → single steer send, never a bare send", pi.calls.length === 1 && pi.calls[0].mode === "steer");
      check("N1(c): no warning/error on the happy steer path", log.warns.length === 0 && log.errors.length === 0);
    }

    // (d) a non-already-processing bare rejection is dropped, NOT retried as steer.
    {
      const pi = makePi((mode) => (mode === "bare" ? "reject-other" : "resolve"));
      const log = makeLog();
      deliverToAgent(pi, "hi", false, log);
      await flush();
      check("N1(d): unrelated failure not retried as steer", pi.calls.length === 1 && pi.calls[0].mode === "bare");
      check("N1(d): unrelated failure dropped loudly", log.errors.some((m) => /DROPPING/.test(m)), log.errors.join("|"));
    }

    // (e) belt-and-braces: a SYNCHRONOUS throw on the bare send still steers.
    {
      const pi = makePi((mode) => (mode === "bare" ? "throw-already" : "resolve"));
      const log = makeLog();
      let threw = false;
      try {
        deliverToAgent(pi, "hi", false, log);
      } catch {
        threw = true;
      }
      await flush();
      check("N1(e): sync throw on bare send does not escape", threw === false);
      check("N1(e): sync throw retried as steer → success", pi.calls.length === 2 && pi.calls[1].mode === "steer" && log.errors.length === 0);
    }
  }

  // ---- Part 6: FB13 — subagent job-card event→CLI mapping (jobcards.ts) -----
  console.log("Part 6: job cards (FB13 subagent → hotline job)");
  {
    const mod = await jiti.import(path.join(root, "src/jobcards.ts"));
    const { titleFrom, failedState, startArgs, doneArgs, updateArgs, wireJobCards } = mod;

    // Pure mappers.
    check("title uses description", titleFrom("do the thing", "researcher") === "do the thing");
    check("title falls back to type", titleFrom("", "researcher") === "researcher");
    check("title falls back to a default", titleFrom(undefined, undefined) === "subagent");
    {
      const long = "x".repeat(200);
      const t = titleFrom(long, "researcher");
      check("title truncates to <=80 with ellipsis", t.length === 80 && t.endsWith("…"), `len ${t.length}`);
    }
    check("failedState maps error→err", failedState("error") === "err");
    check("failedState maps aborted→cancelled", failedState("aborted") === "cancelled");
    check("failedState maps stopped→cancelled", failedState("stopped") === "cancelled");
    check(
      "startArgs shape",
      JSON.stringify(startArgs("c1", "b1", "T")) ===
        JSON.stringify(["job", "start", "--cookie", "c1", "--batch", "b1", "--title", "T"]),
    );
    check(
      "doneArgs always carries --batch (mis-card bug guard)",
      JSON.stringify(doneArgs("c1", "b1", "ok")).includes('"--batch","b1"'),
    );
    check(
      "updateArgs shape",
      JSON.stringify(updateArgs("c1", "d")) === JSON.stringify(["job", "update", "--cookie", "c1", "--detail", "d"]),
    );

    // Wiring: a fake bus + fake spawn capture the emitted argv end to end.
    function makeBus() {
      const handlers = new Map();
      return {
        events: {
          on(channel, handler) {
            handlers.set(channel, handler);
            return () => handlers.delete(channel);
          },
        },
        emit(channel, data) {
          const h = handlers.get(channel);
          if (h) h(data);
        },
        size: () => handlers.size,
      };
    }
    const spawned = [];
    const fakeSpawn = (command, args) => {
      spawned.push({ command, args });
      return { on() {}, unref() {} };
    };
    const silent = { info() {}, warn() {} };

    const bus = makeBus();
    const dispose = wireJobCards(bus, { binary: "hotline", batch: "sess-42", spawn: fakeSpawn, log: silent });

    bus.emit("subagents:started", { id: "a1", type: "researcher", description: "look into X" });
    check("started → job start", spawned.length === 1 && spawned[0].args[1] === "start");
    check("start cookie=id", spawned[0].args.includes("a1"));
    check("start batch=session id", spawned[0].args[spawned[0].args.indexOf("--batch") + 1] === "sess-42");
    check("start title=description", spawned[0].args[spawned[0].args.indexOf("--title") + 1] === "look into X");

    bus.emit("subagents:completed", { id: "a1", status: "done" });
    check("completed → job done ok", spawned[1]?.args.includes("done") && spawned[1].args.includes("ok"));
    check("done carries --batch", spawned[1].args[spawned[1].args.indexOf("--batch") + 1] === "sess-42");

    bus.emit("subagents:failed", { id: "a2", status: "error" });
    check("failed(error) → done err", spawned[2]?.args.includes("err"));
    bus.emit("subagents:failed", { id: "a3", status: "aborted" });
    check("failed(aborted) → done cancelled", spawned[3]?.args.includes("cancelled"));

    bus.emit("subagents:compacted", { id: "a1", reason: "auto" });
    check("compacted → job update", spawned[4]?.args.includes("update") && spawned[4].args.includes("--detail"));

    // Events with no id are ignored (no cookie to key on).
    const n = spawned.length;
    bus.emit("subagents:started", { description: "no id" });
    check("started without id is ignored", spawned.length === n);

    // A throwing spawn never escapes the handler (best-effort contract).
    const throwBus = makeBus();
    const throwDispose = wireJobCards(throwBus, {
      binary: "hotline",
      batch: "s",
      spawn: () => {
        throw new Error("spawn boom");
      },
      log: silent,
    });
    let escaped = false;
    try {
      throwBus.emit("subagents:started", { id: "z", description: "d" });
    } catch {
      escaped = true;
    }
    check("a throwing spawn never escapes the handler", escaped === false);
    throwDispose();

    // Dispose unsubscribes everything.
    dispose();
    check("dispose unsubscribes all listeners", bus.size() === 0);
    const afterDispose = spawned.length;
    bus.emit("subagents:started", { id: "late", description: "d" });
    check("no cards emitted after dispose", spawned.length === afterDispose);
  }

  // ---- Part 7: Mission Control handoff loop (missionControl.ts) ------------
  console.log("Part 7: mission control handoff loop (spec §4/§5)");
  {
    const mc = await jiti.import(path.join(root, "src/missionControl.ts"));
    const {
      parseContextCap,
      clampContextCap,
      compactWithCallbacks,
      effectiveCap,
      usageFraction,
      MissionControlLoop,
      COMPACTION_INSTRUCTIONS,
    } = mc;

    // parseContextCap
    check("parseContextCap unset → null", parseContextCap({}) === null);
    check("parseContextCap invalid → null", parseContextCap({ HOTLINE_MC_CONTEXT_CAP: "nope" }) === null);
    check("parseContextCap negative → null", parseContextCap({ HOTLINE_MC_CONTEXT_CAP: "-5" }) === null);
    check("parseContextCap valid", parseContextCap({ HOTLINE_MC_CONTEXT_CAP: "120000" }) === 120000);

    // Live-validation blocker 3: pi cannot compact inside keepRecentTokens. The
    // soft cap must leave a fixed 4096-token compactable margin above that window.
    check("B3: unset soft cap stays unset", clampContextCap(null, 20000) === null);
    check("B3: cap below keep-recent is raised with margin", clampContextCap(12000, 20000) === 24096);
    check("B3: cap at the minimum remains at the minimum", clampContextCap(24096, 20000) === 24096);
    check("B3: cap above keep-recent margin is unchanged", clampContextCap(120000, 20000) === 120000);
    check("B3: unavailable keep-recent leaves the cap unchanged", clampContextCap(12000, null) === 12000);

    check(
      "B4: direct compact insurance demands an exact final handoff block",
      COMPACTION_INSTRUCTIONS.includes("Append exactly this final block") &&
        COMPACTION_INSTRUCTIONS.includes("MISSION HANDOFF\nDoing:"),
    );

    // Pi's public ctx.compact() is callback-based and returns void. Adapt its
    // onError callback into the promise observed by MissionControlLoop.
    {
      let options;
      const pending = compactWithCallbacks(
        { compact: (received) => { options = received; } },
        "focus",
      );
      options.onError(new Error("callback rejection"));
      let detail = "";
      try {
        await pending;
      } catch (err) {
        detail = String(err);
      }
      check("B2: compact onError callback rejects the observed promise", detail.includes("callback rejection"));
      check("B2: compact adapter passes custom instructions", options.customInstructions === "focus");
    }

    // effectiveCap / usageFraction
    check("effectiveCap prefers the soft cap", effectiveCap({ tokens: 1, contextWindow: 200000 }, 100000) === 100000);
    check("effectiveCap falls back to the window", effectiveCap({ tokens: 1, contextWindow: 200000 }, null) === 200000);
    check("usageFraction null tokens → null", usageFraction({ tokens: null, contextWindow: 200000 }, null) === null);
    check("usageFraction against cap", Math.abs(usageFraction({ tokens: 90000, contextWindow: 200000 }, 100000) - 0.9) < 1e-9);

    // A recording actions harness.
    function harness(cap) {
      const events = { nudges: [], compacts: [], handoffTurns: [], mechanical: [] };
      const actions = {
        armNudge: (line) => events.nudges.push(line),
        compact: (ci) => events.compacts.push(ci),
        sendHandoffTurn: (p) => events.handoffTurns.push(p),
        mechanicalHandoff: (s, n) => events.mechanical.push([s, n]),
        log: () => {},
      };
      return { loop: new MissionControlLoop(cap, actions), events };
    }

    // Nudge arms at 80%, re-arms only after +10% growth.
    {
      const { loop, events } = harness(100000);
      loop.onContextUsage({ tokens: 70000, contextWindow: 200000 });
      check("no nudge below 80%", events.nudges.length === 0);
      loop.onContextUsage({ tokens: 82000, contextWindow: 200000 });
      check("nudge arms at 82%", events.nudges.length === 1);
      loop.onContextUsage({ tokens: 85000, contextWindow: 200000 });
      check("no re-nudge inside the 10% step", events.nudges.length === 1);
      loop.onContextUsage({ tokens: 93000, contextWindow: 200000 });
      check("re-nudge after +10% growth", events.nudges.length === 2);
    }

    // Live-validation blocker 1: onContextUsage can arm a handoff turn while
    // agent_settled is already running. The immediate settle callback is still
    // generation 1, so it must not consume work that belongs to generation 2.
    {
      const { loop, events } = harness(100000);
      loop.onContextUsage({ tokens: 101000, contextWindow: 200000 }, 1);
      loop.onHandoffTurnSettled(false, "cap work", 1);
      check("B1: the settle that arms a cap handoff cannot consume it", events.mechanical.length === 0);
      check("B1: compact cannot run before the queued handoff turn", events.compacts.length === 0);
      loop.onHandoffTurnSettled(false, "cap work", 2);
      check("B1: the next settle consumes the handoff turn", events.mechanical.length === 1);
      check("B1: compaction follows the handoff turn's own settle", events.compacts.length === 1);
    }

    // Ride-along: pi checks automatic compaction again after the queued handoff
    // continuation but before agent_settled. Keep cancelling while that handoff
    // is armed, so only its own later settle can fallback and recompact.
    {
      const { loop, events } = harness(100000);
      loop.onContextUsage({ tokens: 95000, contextWindow: 200000 }, 1);
      const first = loop.onBeforeCompact("threshold");
      const beforeSettle = loop.onBeforeCompact("threshold");
      check("ride-along: first automatic compaction is intercepted", first.cancel === true);
      check("ride-along: automatic recheck waits for handoff settle", beforeSettle.cancel === true);
      check("ride-along: automatic recheck launches no compact", events.compacts.length === 0);
      loop.onHandoffTurnSettled(false, "handoff missed", 2);
      check("ride-along: no-handoff fallback runs after handoff settle", events.mechanical.length === 1);
      check("ride-along: handoff settle launches one direct compact", events.compacts.length === 1);
      const direct = loop.onBeforeCompact("manual");
      check("ride-along: direct recompact proceeds", direct.cancel === false);
    }

    // An automatic before_compact intercept can queue the handoff below the soft
    // cap, then have its follow-up settle above the cap. Context enforcement and
    // handoff consumption must not launch two concurrent direct compactions.
    {
      const { loop, events } = harness(100000);
      loop.onContextUsage({ tokens: 95000, contextWindow: 200000 }, 1);
      const intercepted = loop.onBeforeCompact("threshold");
      check("ride-along: automatic compaction is intercepted", intercepted.cancel === true);
      loop.noteHandoffWritten();
      loop.onContextUsage({ tokens: 101000, contextWindow: 200000 }, 2);
      loop.onHandoffTurnSettled(true, "handoff done", 2);
      check(
        "ride-along: over-cap handoff settle requests exactly one compaction",
        events.compacts.length === 1,
        `compacts ${events.compacts.length}`,
      );
    }

    // Soft cap (P2-B): crossing the cap runs a real handoff turn FIRST, then
    // compaction — pi reports "manual" for our own compact() so before_compact
    // can't run layer 3; the cap path must drive the turn itself.
    {
      const { loop, events } = harness(100000);
      loop.onContextUsage({ tokens: 99000, contextWindow: 200000 });
      check("no handoff turn below the cap", events.handoffTurns.length === 0);
      check("no compact below the cap", events.compacts.length === 0);
      loop.onContextUsage({ tokens: 101000, contextWindow: 200000 });
      check("cap crossed sends a handoff turn first", events.handoffTurns.length === 1);
      check("compaction not fired yet (waits for the turn)", events.compacts.length === 0);
      loop.onContextUsage({ tokens: 102000, contextWindow: 200000 });
      check("handoff turn not re-sent while still over the cap", events.handoffTurns.length === 1);
      // The turn settles; the agent wrote nothing → mechanical fallback + compact.
      loop.onHandoffTurnSettled(false, "cap work");
      check("cap path fires the mechanical fallback when nothing written", events.mechanical.length === 1);
      check("cap path compacts after the handoff turn", events.compacts.length === 1);
    }

    // Cap path + the agent writes its own handoff during the turn → no fallback.
    {
      const { loop, events } = harness(100000);
      loop.onContextUsage({ tokens: 101000, contextWindow: 200000 });
      check("cap crossed sends the handoff turn", events.handoffTurns.length === 1);
      loop.noteHandoffWritten();
      loop.onHandoffTurnSettled(true, "wrote it");
      check("no mechanical fallback when the agent wrote a handoff (cap path)", events.mechanical.length === 0);
      check("cap path still compacts after the turn", events.compacts.length === 1);
    }

    // Cap path + a FRESH handoff already on disk → straight to compaction, no turn.
    {
      const { loop, events } = harness(100000);
      loop.onContextUsage({ tokens: 96000, contextWindow: 200000 });
      loop.noteHandoffWritten(); // written inside the last 25% of the cap
      loop.onContextUsage({ tokens: 101000, contextWindow: 200000 });
      check("fresh handoff → cap path skips the handoff turn", events.handoffTurns.length === 0);
      check("fresh handoff → cap path compacts directly", events.compacts.length === 1);
    }

    // No cap → no cap-driven compaction, but nudge still works against the window.
    {
      const { loop, events } = harness(null);
      check("hasCap false when unset", loop.hasCap() === false);
      loop.onContextUsage({ tokens: 170000, contextWindow: 200000 });
      check("nudge fires against the window with no cap", events.nudges.length === 1);
      check("no compact without a cap", events.compacts.length === 0);
    }

    // The cancel → handoff-turn → recompact loop, agent writes a handoff.
    {
      const { loop, events } = harness(100000);
      const first = loop.onBeforeCompact();
      check("first before_compact cancels", first.cancel === true);
      check(
        "B4: before_compact does not claim an unsupported instruction override",
        !Object.hasOwn(first, "customInstructions"),
      );
      check("a real handoff turn is sent", events.handoffTurns.length === 1);
      // Agent wrote a handoff during the turn.
      loop.noteHandoffWritten();
      loop.onHandoffTurnSettled(true, "last msg");
      check("no mechanical fallback when the agent wrote a handoff", events.mechanical.length === 0);
      check("compaction re-triggered after the handoff turn", events.compacts.length === 1);
      const second = loop.onBeforeCompact();
      check("second before_compact proceeds (no re-cancel)", second.cancel === false);
      loop.onCompacted();
    }

    // The mechanical fallback when the agent writes nothing.
    {
      const { loop, events } = harness(100000);
      // Below the cap so this doesn't itself trigger a cap-compaction; still sets
      // lastTokens for the P3 fallback state string.
      loop.onContextUsage({ tokens: 95000, contextWindow: 200000 });
      loop.onBeforeCompact();
      loop.onHandoffTurnSettled(false, "did some work here");
      check("mechanical fallback fires when no handoff written", events.mechanical.length === 1);
      check("fallback state mentions the last message", events.mechanical[0][0].includes("did some work"));
      check("fallback state includes the token count (P3)", events.mechanical[0][0].includes("95000 tokens"));
      check("fallback still re-triggers compaction", events.compacts.length === 1);
    }

    // Finding: manual /compact must NOT be hijacked for a handoff turn.
    {
      const { loop, events } = harness(100000);
      const r = loop.onBeforeCompact("manual");
      check("manual compaction is never cancelled", r.cancel === false);
      check("manual compaction sends no handoff turn", events.handoffTurns.length === 0);
      // Automatic (threshold) still gets the intercept.
      const r2 = loop.onBeforeCompact("threshold");
      check("threshold compaction is still cancelled for the handoff turn", r2.cancel === true);
    }

    // P3: a STALE handoff (written well before the last 25% of the window) must
    // NOT suppress the intercept — the turn still runs so the summary is fresh.
    {
      const { loop, events } = harness(100000);
      loop.onContextUsage({ tokens: 40000, contextWindow: 200000 });
      loop.noteHandoffWritten(); // handoff at 40k — far from the compaction point
      loop.onContextUsage({ tokens: 98000, contextWindow: 200000 });
      const r = loop.onBeforeCompact("threshold");
      check("stale handoff still triggers the handoff turn (P3)", r.cancel === true);
      check("stale handoff → a real handoff turn is sent", events.handoffTurns.length === 1);
    }

    // P3: a FRESH handoff (written inside the last 25%) still suppresses the turn.
    {
      const { loop, events } = harness(100000);
      loop.onContextUsage({ tokens: 90000, contextWindow: 200000 });
      loop.noteHandoffWritten(); // handoff at 90k — inside the last 25% of the cap
      const r = loop.onBeforeCompact("threshold");
      check("fresh handoff suppresses the intercept (P3)", r.cancel === false);
      check("fresh handoff sends no handoff turn (P3)", events.handoffTurns.length === 0);
    }

    // Finding: stuck flag — if the re-triggered compact() no-ops (throws in prod,
    // swallowed by the action) its before_compact never fires, so a later settle
    // must NOT re-fire the mechanical fallback + compaction forever.
    {
      const { loop, events } = harness(100000);
      loop.onBeforeCompact("threshold");
      loop.onHandoffTurnSettled(false, "work"); // mechanical #1, compact #1
      // The recompact's before_compact never arrives; the next turn settles again.
      loop.onHandoffTurnSettled(false, "more work");
      check("no repeat mechanical when the recompact never fires", events.mechanical.length === 1);
      check("no repeat compaction when the recompact never fires", events.compacts.length === 1);
    }

    // Live-validation blocker 2: ctx.compact() is async. A rejection must be
    // observed, logged, and reset the cap/recompact latches so usage still above
    // the cap can retry on a later settle. Keep a local rejection handler here so
    // the old implementation fails by latch behavior rather than process-level
    // unhandled-rejection policy.
    {
      let attempts = 0;
      const warnings = [];
      const infos = [];
      const events = { handoffTurns: [], mechanical: [] };
      const actions = {
        armNudge() {},
        compact() {
          attempts++;
          const rejected = Promise.reject(new Error("Nothing to compact (session too small)"));
          rejected.catch(() => {});
          return rejected;
        },
        sendHandoffTurn: (p) => events.handoffTurns.push(p),
        mechanicalHandoff: (s, n) => events.mechanical.push([s, n]),
        log: (m) => infos.push(m),
        warn: (m) => warnings.push(m),
      };
      const loop = new MissionControlLoop(100000, actions);
      loop.onContextUsage({ tokens: 101000, contextWindow: 200000 }, 1);
      loop.onHandoffTurnSettled(false, "cap work", 1);
      check("B2: compact is not allowed to abort the queued handoff turn", attempts === 0);
      loop.onHandoffTurnSettled(false, "cap work", 2);
      check("B2: compact starts after the handoff turn completes", attempts === 1);
      await new Promise((resolve) => setImmediate(resolve));
      loop.onContextUsage({ tokens: 120000, contextWindow: 200000 }, 3);
      check("B2: rejected compact resets state so over-cap usage retries", attempts === 2, `attempts ${attempts}`);
      check("B2: benign no-op logs the real compact error", infos.some((m) => m.includes("Nothing to compact")), infos.join("|"));
      check("B2: benign no-op is not escalated as a warning", warnings.length === 0, warnings.join("|"));
    }
  }

  // ---- Part 8: pi hot model/effort apply (piapply.ts) ----------------------
  // The pi counterpart of the claude-sdk sdk_apply handler: the box forwards a
  // set_sdk_config as a sdk_apply notification, we apply it to the LIVE pi
  // session through ExtensionAPI and answer on sdk_apply_result. Everything is
  // driven through injected deps, so no pi session is needed.
  console.log("Part 8: pi hot apply (sdk_apply → setModel/setThinkingLevel)");
  {
    const mod = await jiti.import(path.join(root, "src/piapply.ts"));
    const { createPiApplyHandler, planEffort, IdentityAnnouncer, PI_THINKING_LEVELS } = mod;

    const silentLog = { info() {}, warn() {}, error() {} };

    // A registry of two real fleet models: what the boxes actually run.
    const CATALOG = {
      "gpt-5.6-sol": { provider: "openai-codex", id: "gpt-5.6-sol" },
      "openai-codex/gpt-5.6-sol": { provider: "openai-codex", id: "gpt-5.6-sol" },
      "glm-5.2": { provider: "zai", id: "glm-5.2" },
      "zai/glm-5.2": { provider: "zai", id: "glm-5.2" },
      "glm-5.2:high": { provider: "zai", id: "glm-5.2", thinking: "high" },
    };

    /** Build a handler over a scriptable fake pi. */
    function harness(over = {}) {
      const calls = { setModel: [], setThinkingLevel: [], applied: [], results: [] };
      let level = over.level ?? "medium";
      const deps = {
        resolveModel(pattern) {
          if (over.resolveThrows) throw new Error("registry exploded");
          const hit = CATALOG[pattern];
          if (!hit) return null;
          return {
            model: { provider: hit.provider, id: hit.id },
            provider: hit.provider,
            id: hit.id,
            canonical: `${hit.provider}/${hit.id}`,
            thinkingLevel: hit.thinking,
          };
        },
        async setModel(m) {
          calls.setModel.push(m);
          return over.setModelReturns ?? true;
        },
        getThinkingLevel: () => level,
        setThinkingLevel(l) {
          calls.setThinkingLevel.push(l);
          if (over.setThinkingThrows) throw new Error("thinking rejected");
          // Model capability clamp: the fake caps at `clampTo` when given.
          level = over.clampTo ?? l;
        },
        getSession: () => (over.ready === false ? null : (over.session?.() ?? 1)),
        notifyResult: (r) => calls.results.push(r),
        onApplied: (a) => calls.applied.push(a),
        log: silentLog,
      };
      const wrapper = { setModel: (m) => deps.setModel(m) };
      const bound = { ...deps, setModel: (m) => wrapper.setModel(m) };
      return {
        handle: createPiApplyHandler(bound),
        calls,
        currentLevel: () => level,
        deps: wrapper,
      };
    }

    // -- the effort mapping table ------------------------------------------
    check("effort table: our five wire levels are pi levels verbatim",
      ["low", "medium", "high", "xhigh", "max"].every(
        (e) => planEffort(e).kind === "level" && planEffort(e).level === e,
      ));
    check("effort table: off/minimal accepted (pi TUI can set them)",
      planEffort("off").kind === "level" && planEffort("minimal").kind === "level");
    check("effort table: \"\" is a clear, not a level", planEffort("").kind === "clear");
    check("effort table: a raw token budget is INVALID on pi", planEffort("32000").kind === "invalid");
    check("effort table: junk is invalid", planEffort("xtreme").kind === "invalid");
    check("effort table: covers exactly pi's ThinkingLevel union",
      PI_THINKING_LEVELS.join(",") === "off,minimal,low,medium,high,xhigh,max");

    // -- the call-timeout override reaches tool calls (sol review #8) -------
    // mcpCallTool takes the timeout as a DEFAULTED parameter, so a caller that
    // omits it compiles, runs, and silently ignores HOTLINE_PI_CALL_TIMEOUT_MS.
    // That is exactly what the core extraction did to this harness: the timeout
    // family was resolved for ChildManager and dropped at the tool-call site,
    // so an operator override on a slow link parsed, logged, and did nothing.
    // A behavioural test cannot reach inside the pi extension closure, so guard
    // the shape: every mcpCallTool call site must pass a timeout.
    {
      const src = fs.readFileSync(new URL("../src/index.ts", import.meta.url), "utf8");
      const sites = [...src.matchAll(/mcpCallTool\(([^;]*?)\)/gs)];
      check("timeout: there is at least one mcpCallTool site to guard", sites.length > 0);
      const bare = sites.filter((m) => m[1].split(",").length < 4);
      check(
        "timeout: every mcpCallTool site passes an explicit timeout, not core's default",
        bare.length === 0,
        bare.map((m) => m[0]).join(" | "),
      );
      check(
        "timeout: the passed value is the resolved HOTLINE_PI family, not a literal",
        sites.every((m) => m[1].includes("timeouts.callTimeoutMs")),
        sites.map((m) => m[0]).join(" | "),
      );
      check(
        "timeout: the family is resolved once for both the child manager and tool calls",
        /const timeouts = timeoutsFromEnv\("HOTLINE_PI"\)/.test(src),
      );
    }

    // -- generation fencing (sol review #6, pi half) ------------------------
    // A pi apply awaits setModel, and pi can shut the session down inside that
    // await. A single readiness check taken at entry then lets setThinkingLevel
    // and the harness_info restamp run against a session that is already gone.
    {
      let generation = 1;
      const h = harness({ level: "medium", session: () => generation });
      // session_shutdown lands while setModel is in flight.
      const inner = h.deps.setModel;
      h.deps.setModel = async (m) => {
        const ok = await inner(m);
        generation += 1;
        return ok;
      };
      await h.handle({ rid: "gen1", model: "gpt-5.6-sol", effort: "high" });
      const r = h.calls.results[0];
      check("gen: a session replaced mid-apply answers no_session",
        r && r.ok === false && r.code === "no_session", JSON.stringify(r));
      check("gen: the dead session never got a thinking level",
        h.calls.setThinkingLevel.length === 0, JSON.stringify(h.calls.setThinkingLevel));
      check("gen: harness_info was not restamped for a session that is gone",
        h.calls.applied.length === 0, JSON.stringify(h.calls.applied));
      check("gen: exactly one result", h.calls.results.length === 1);
    }
    {
      // The fence must not brake a normal apply.
      const h = harness({ level: "medium", session: () => 42 });
      await h.handle({ rid: "gen2", model: "gpt-5.6-sol", effort: "high" });
      check("gen: an unchanged session applies normally",
        h.calls.results[0]?.ok === true && h.calls.applied.length === 1);
    }

    // -- success: model + effort, canonical echo ----------------------------
    {
      const h = harness({ level: "medium" });
      await h.handle({ rid: "r1", model: "gpt-5.6-sol", effort: "high" });
      const r = h.calls.results[0];
      check("apply ok", r && r.ok === true, JSON.stringify(r));
      check("apply echoes the CANONICAL provider/id, not the request",
        r.model === "openai-codex/gpt-5.6-sol", r.model);
      check("apply echoes the effort", r.effort === "high", r.effort);
      check("apply never reports unverified (pi resolution is authoritative)",
        r.unverified === undefined);
      check("setModel got the resolved Model object",
        h.calls.setModel.length === 1 && h.calls.setModel[0].id === "gpt-5.6-sol");
      check("model is applied BEFORE the level (the clamp reads the new model)",
        h.calls.setThinkingLevel.length === 1 && h.calls.setThinkingLevel[0] === "high");
      check("onApplied fires once, before the result",
        h.calls.applied.length === 1 &&
          h.calls.applied[0].model === "openai-codex/gpt-5.6-sol" &&
          h.calls.applied[0].effort === "high");
      check("exactly one result", h.calls.results.length === 1);
    }

    // -- unknown model -------------------------------------------------------
    {
      const h = harness();
      await h.handle({ rid: "r2", model: "gpt-9-nope" });
      const r = h.calls.results[0];
      check("unknown model → unknown_model", r.ok === false && r.code === "unknown_model", JSON.stringify(r));
      check("unknown model never touches the session", h.calls.setModel.length === 0);
      check("unknown model restamps nothing", h.calls.applied.length === 0);
    }

    // -- setModel returns false (the pi-only signal) -------------------------
    {
      const h = harness({ setModelReturns: false });
      await h.handle({ rid: "r3", model: "zai/glm-5.2", effort: "high" });
      const r = h.calls.results[0];
      check("setModel false → no_api_key (distinct from unknown_model)",
        r.ok === false && r.code === "no_api_key", JSON.stringify(r));
      check("no_api_key names the provider", /zai/.test(r.detail ?? ""), r.detail);
      check("no_api_key does not go on to set the level", h.calls.setThinkingLevel.length === 0);
      check("no_api_key restamps nothing", h.calls.applied.length === 0);
    }

    // -- effort clamp: report what pi actually settled on ---------------------
    {
      const h = harness({ level: "low", clampTo: "high" });
      await h.handle({ rid: "r4", effort: "max" });
      const r = h.calls.results[0];
      check("clamped effort is READ BACK, not echoed from the request",
        r.ok === true && r.effort === "high", JSON.stringify(r));
      check("clamped effort is what gets mirrored/persisted",
        h.calls.applied[0].effort === "high");
      check("clamp still asked for the requested level",
        h.calls.setThinkingLevel[0] === "max");
    }

    // -- partial failure: model landed, effort threw --------------------------
    {
      const h = harness({ setThinkingThrows: true });
      await h.handle({ rid: "r5", model: "gpt-5.6-sol", effort: "xhigh" });
      const r = h.calls.results[0];
      check("partial failure reports apply_failed", r.ok === false && r.code === "apply_failed", JSON.stringify(r));
      check("partial failure surfaces the throw's message", /thinking rejected/.test(r.detail ?? ""), r.detail);
      check("partial failure STILL restamps the part that landed",
        h.calls.applied.length === 1 && h.calls.applied[0].model === "openai-codex/gpt-5.6-sol");
      check("partial failure does not claim the effort landed",
        h.calls.applied[0].effort === undefined);
    }

    // -- invalid effort is refused before the session is touched --------------
    {
      const h = harness();
      await h.handle({ rid: "r6", model: "gpt-5.6-sol", effort: "32000" });
      const r = h.calls.results[0];
      check("raw token budget → invalid_effort on pi", r.ok === false && r.code === "invalid_effort", JSON.stringify(r));
      check("invalid effort refuses BEFORE applying the model", h.calls.setModel.length === 0);
    }

    // -- clear ("") unpins without touching the live session ------------------
    {
      const h = harness({ level: "medium" });
      await h.handle({ rid: "r7", model: "", effort: "" });
      const r = h.calls.results[0];
      check("clear is ok", r.ok === true, JSON.stringify(r));
      check("clear echoes empty strings so the box removes the .env lines",
        r.model === "" && r.effort === "");
      check("clear applies nothing live (pi always has a model and a level)",
        h.calls.setModel.length === 0 && h.calls.setThinkingLevel.length === 0);
      check("clear leaves the live level alone", h.currentLevel() === "medium");
    }

    // -- a ":level" suffix inside the model pattern ---------------------------
    {
      const h = harness({ level: "low" });
      await h.handle({ rid: "r8", model: "glm-5.2:high" });
      check("model pattern's :level is applied when no effort field is sent",
        h.calls.setThinkingLevel.length === 1 && h.calls.setThinkingLevel[0] === "high");
      check("model pattern's :level does not fabricate an effort echo",
        h.calls.results[0].effort === undefined);
    }
    {
      const h = harness({ level: "low" });
      await h.handle({ rid: "r9", model: "glm-5.2:high", effort: "low" });
      check("an explicit effort field beats the pattern's :level suffix",
        h.calls.setThinkingLevel.length === 1 && h.calls.setThinkingLevel[0] === "low");
    }

    // -- no session / malformed ----------------------------------------------
    {
      const h = harness({ ready: false });
      await h.handle({ rid: "r10", model: "gpt-5.6-sol" });
      check("not ready → no_session", h.calls.results[0].code === "no_session");
    }
    {
      const h = harness();
      await h.handle({ model: "gpt-5.6-sol" });
      check("no rid → dropped, never answered", h.calls.results.length === 0);
      await h.handle({ rid: "r11" });
      check("no field → dropped, never answered", h.calls.results.length === 0);
    }
    {
      const h = harness({ resolveThrows: true });
      await h.handle({ rid: "r12", model: "gpt-5.6-sol" });
      check("a registry throw is apply_failed, not unknown_model",
        h.calls.results[0].code === "apply_failed");
    }

    // -- bidirectional sync: the restamp-storm guard ---------------------------
    {
      const sent = [];
      let childUp = true;
      const a = new IdentityAnnouncer((info) => {
        if (!childUp) return false;
        sent.push(info);
        return true;
      });
      check("first announce emits", a.announce("zai/glm-5.2", "high") === true);
      check("an identical restamp is deduped",
        a.announce("zai/glm-5.2", "high") === false && sent.length === 1);
      check("a real model change emits", a.announce("openai-codex/gpt-5.6-sol", "high") === true);
      check("a real level change emits", a.announce("openai-codex/gpt-5.6-sol", "max") === true);
      check("three emits total", sent.length === 3, `got ${sent.length}`);
      check("payload carries harness + both fields",
        sent[0].harness === "pi" && sent[0].model === "zai/glm-5.2" && sent[0].effort === "high");
      check("force re-emits an unchanged identity (a respawned child needs it)",
        a.announce("openai-codex/gpt-5.6-sol", "max", true) === true && sent.length === 4);
      // A one-apply storm: model_select + thinking_level_select + the explicit
      // post-apply restamp all announce the same identity — one emit, not three.
      const before = sent.length;
      a.announce("zai/glm-5.2", "low");
      a.announce("zai/glm-5.2", "low");
      a.announce("zai/glm-5.2", "low");
      check("one apply's three restamps collapse to a single harness_info",
        sent.length === before + 1, `got ${sent.length - before}`);
      // A dropped announcement (child mid-respawn) must not be recorded.
      childUp = false;
      check("a drop reports false", a.announce("zai/glm-5.1", "low") === false);
      childUp = true;
      check("a dropped identity is re-sent once the child is back",
        a.announce("zai/glm-5.1", "low") === true);
      // Session boundaries forget the last announcement.
      a.reset();
      check("reset forgets, so the next session re-announces",
        a.announce("zai/glm-5.1", "low") === true);
    }
  }


  // ---- Part 9: model catalog (modelcatalog.ts) -----------------------------
  // Where the app's model rows come from. Two tiers that are deliberately
  // different sets: the catalog is the operator's FILTERED cycling scope,
  // reconstructed through pi's own resolver; the free-text escape (Part 8's
  // hot-apply path) reaches the FULL registry. Everything here is driven
  // through injected deps, so no pi session and no real registry are needed.
  console.log("");
  console.log("Part 9: model catalog (harness_catalog enumeration)");
  {
    const mod = await jiti.import(path.join(root, "src/modelcatalog.ts"));
    const { buildCatalog, parsePiModels, canonicalId, modelLabel, MAX_CATALOG_MODELS } = mod;

    const silentLog = { info() {}, warn() {}, error() {} };
    const warnLog = () => {
      const warns = [];
      return { log: { info() {}, warn: (m) => warns.push(m), error() {} }, warns };
    };

    // A registry standing in for a real box: two providers with credentials,
    // one without.
    const SOL = { provider: "openai-codex", id: "gpt-5.6-sol", name: "Sol" };
    const LUNA = { provider: "openai-codex", id: "gpt-5.6-luna", name: "Luna" };
    const OPUS = { provider: "anthropic", id: "claude-opus-4-8", name: "Opus 4.8" };
    const GLM = { provider: "zai", id: "glm-5.2", name: "GLM 5.2" };
    const GEMINI = { provider: "google", id: "gemini-3-pro", name: "Gemini 3 Pro" };
    const ALL = [SOL, LUNA, OPUS, GLM, GEMINI];
    // getAvailable() = auth configured. google is in the registry with no key,
    // so it is absent here — exactly how pi's registry behaves.
    const AVAILABLE = [SOL, LUNA, OPUS, GLM];
    const hasAuth = (m) => AVAILABLE.some((a) => a.provider === m.provider && a.id === m.id);

    /** A resolveScope that filters AVAILABLE by a naive glob/substring, which is
     * enough to mirror the shape of pi's resolveModelScopeWithDiagnostics. */
    function resolveScope(patterns) {
      const scopedModels = [];
      const diagnostics = [];
      for (const raw of patterns) {
        const pattern = raw.replace(/:(off|minimal|low|medium|high|xhigh|max)$/, "");
        const rx = new RegExp("^" + pattern.split("*").map((x) => x.replace(/[.+?^${}()|[\]\\]/g, "\\$&")).join(".*") + "$", "i");
        const hits = AVAILABLE.filter((m) => rx.test(`${m.provider}/${m.id}`) || rx.test(m.id));
        if (hits.length === 0) {
          diagnostics.push({ message: `No models match pattern "${raw}"` });
          continue;
        }
        for (const m of hits) if (!scopedModels.find((s) => s.model === m)) scopedModels.push({ model: m });
      }
      return Promise.resolve({ scopedModels, diagnostics });
    }

    const deps = (over = {}) => ({
      patterns: over.patterns ?? [],
      enabledModels: over.enabledModels ?? (() => undefined),
      resolveScope: over.resolveScope ?? resolveScope,
      getAvailable: over.getAvailable ?? (() => AVAILABLE),
      hasConfiguredAuth: over.hasConfiguredAuth ?? hasAuth,
      log: over.log ?? silentLog,
    });

    // -- pattern parsing: byte-parity with the Go ParsePiModels rule ---------
    check("patterns: comma split + trim", 
      parsePiModels("a, b ,c").join("|") === "a|b|c");
    check("patterns: blanks dropped", parsePiModels("a,,b,").join("|") === "a|b");
    check("patterns: unset is empty", parsePiModels(undefined).length === 0 && parsePiModels("").length === 0);
    check("patterns: an over-long pattern is DROPPED, never truncated",
      parsePiModels("ok," + "x".repeat(65)).join("|") === "ok");
    check("patterns: count capped at 32",
      parsePiModels(Array.from({ length: 40 }, (_, i) => `m${i}`).join(",")).length === 32);
    check("patterns: globs and :level suffixes survive verbatim",
      parsePiModels("anthropic/*, *sonnet* , glm-5.2:high").join("|") === "anthropic/*|*sonnet*|glm-5.2:high");

    // -- canonical id + label ------------------------------------------------
    check("canonical id is provider/id", canonicalId(SOL) === "openai-codex/gpt-5.6-sol");
    check("canonical id of a provider-less model is the bare id",
      canonicalId({ id: "solo" }) === "solo");
    check("a model with no id is not selectable", canonicalId({ provider: "x" }) === null);
    check("label prefers pi's own name", modelLabel(SOL) === "Sol");
    check("label falls back to the BARE id, not the canonical form",
      modelLabel({ provider: "zai", id: "glm-5.2" }) === "glm-5.2");

    // -- tier 1: the FILTERED scope is what the catalog reports ---------------
    {
      const cat = await buildCatalog(deps({ patterns: ["openai-codex/*"] }));
      check("scope: source is 'models' when the operator set one", cat.source === "models");
      check("scope: only the scoped models are offered",
        cat.models.map((m) => m.id).join("|") === "openai-codex/gpt-5.6-sol|openai-codex/gpt-5.6-luna");
      check("scope: entries carry canonical ids the app can send back",
        cat.models[0].id === "openai-codex/gpt-5.6-sol");
      check("scope: entries carry pi's display labels", cat.models[0].label === "Sol");
      check("scope: harness is stamped", cat.harness === "pi");
      check("scope: an unscoped box is NOT what this reports", cat.models.length === 2);
    }
    {
      const cat = await buildCatalog(deps({ patterns: ["glm-5.2:high", "anthropic/claude-opus-4-8"] }));
      check("scope: a :level suffix does not stop a pattern resolving",
        cat.models.map((m) => m.id).join("|") === "zai/glm-5.2|anthropic/claude-opus-4-8");
    }

    // -- availability --------------------------------------------------------
    {
      const cat = await buildCatalog(deps({ patterns: ["openai-codex/*"] }));
      check("availability: a scoped model with auth is available",
        cat.models.every((m) => m.available === true));
    }
    {
      // A registry whose scope resolution let through a model with no key (pi
      // pre-filters today; the flag must still tell the truth if it stops).
      const cat = await buildCatalog(deps({
        patterns: ["*"],
        resolveScope: () => Promise.resolve({ scopedModels: ALL.map((m) => ({ model: m })), diagnostics: [] }),
      }));
      const gem = cat.models.find((m) => m.id === "google/gemini-3-pro");
      check("availability: computed per entry from hasConfiguredAuth, not assumed",
        gem !== undefined && gem.available === false);
      check("availability: the keyed ones are still available",
        cat.models.find((m) => m.id === "zai/glm-5.2").available === true);
    }
    {
      const { log, warns } = warnLog();
      const cat = await buildCatalog(deps({
        patterns: ["openai-codex/*"],
        hasConfiguredAuth: () => { throw new Error("keyring locked"); },
        log,
      }));
      check("availability: an auth probe that THROWS is not evidence of a key",
        cat.models.every((m) => m.available === false));
      check("availability: the throw is logged, not swallowed", warns.some((w) => w.includes("keyring locked")));
    }

    // -- precedence: models -> settings -> available --------------------------
    {
      const cat = await buildCatalog(deps({ enabledModels: () => ["anthropic/*"] }));
      check("precedence: with no --models scope, pi's enabledModels setting is used",
        cat.source === "settings" && cat.models.map((m) => m.id).join("|") === "anthropic/claude-opus-4-8");
    }
    {
      const cat = await buildCatalog(deps({ patterns: ["openai-codex/*"], enabledModels: () => ["anthropic/*"] }));
      check("precedence: an explicit scope BEATS the setting (pi's own `parsed.models ?? settings`)",
        cat.source === "models" && cat.models.length === 2);
    }
    {
      const cat = await buildCatalog(deps());
      check("precedence: unscoped reports every model with auth",
        cat.source === "available" && cat.models.length === AVAILABLE.length);
      check("precedence: the unkeyed registry model is NOT in the unscoped list",
        !cat.models.some((m) => m.id === "google/gemini-3-pro"));
    }
    {
      const { log, warns } = warnLog();
      const cat = await buildCatalog(deps({ enabledModels: () => { throw new Error("settings.json is junk"); }, log }));
      check("precedence: a settings read that throws degrades to available, never empty",
        cat.source === "available" && cat.models.length === AVAILABLE.length);
      check("precedence: the settings failure is logged", warns.some((w) => w.includes("junk")));
    }

    // -- a scope matching nothing is NOT an empty menu ------------------------
    {
      const { log, warns } = warnLog();
      const cat = await buildCatalog(deps({ patterns: ["nope/*"], log }));
      check("empty scope: pi's cycleModel ignores an empty scope, so we report available too",
        cat.source === "available" && cat.models.length === AVAILABLE.length);
      check("empty scope: the operator's typo is surfaced in the log",
        warns.some((w) => w.includes("No models match")));
    }

    // -- de-dup, cap, and failure degradation --------------------------------
    {
      const cat = await buildCatalog(deps({
        patterns: ["openai-codex/*", "openai-codex/gpt-5.6-sol"],
      }));
      check("dedup: a model matched by two patterns appears once",
        cat.models.filter((m) => m.id === "openai-codex/gpt-5.6-sol").length === 1);
    }
    {
      const many = Array.from({ length: MAX_CATALOG_MODELS + 10 }, (_, i) => ({
        provider: "p", id: `m${i}`, name: `M${i}`,
      }));
      const cat = await buildCatalog(deps({ getAvailable: () => many, hasConfiguredAuth: () => true }));
      check("cap: the list is cut at the cap", cat.models.length === MAX_CATALOG_MODELS);
      check("cap: a cut list says so", cat.truncated === true);
      check("cap: an uncut list does not carry the flag",
        (await buildCatalog(deps())).truncated === undefined);
    }
    {
      const { log, warns } = warnLog();
      const cat = await buildCatalog(deps({
        getAvailable: () => { throw new Error("registry exploded"); }, log,
      }));
      check("degrade: a registry failure yields an EMPTY catalog, never a throw",
        cat.models.length === 0);
      check("degrade: an empty catalog is the app's fallback signal, and it is logged",
        warns.some((w) => w.includes("registry exploded")));
    }

    // -- tier 2: free text reaches PAST the catalog ---------------------------
    // The operator ask this whole amendment exists for: the rows are the
    // filtered scope, but typing a model must work for anything in the FULL
    // registry that has a key — and must be REFUSED, not silently applied, for
    // one that has none. Wires the real hot-apply handler over the same
    // registry the catalog was built from, so the two tiers are proven to be
    // different sets against one source of truth.
    {
      const apply = await jiti.import(path.join(root, "src/piapply.ts"));
      const cat = await buildCatalog(deps({ patterns: ["openai-codex/*"] }));
      const offered = new Set(cat.models.map((m) => m.id));
      check("two-tier: the catalog does NOT offer glm-5.2", !offered.has("zai/glm-5.2"));
      check("two-tier: the catalog does NOT offer the unkeyed gemini", !offered.has("google/gemini-3-pro"));

      const results = [];
      // resolveModel spans the FULL registry (pi's resolveCliModel uses
      // getAll(), deliberately including models without pre-configured auth);
      // setModel is the key check, returning false exactly when there is none.
      const handle = apply.createPiApplyHandler({
        resolveModel(pattern) {
          const hit = ALL.find((m) => `${m.provider}/${m.id}` === pattern || m.id === pattern);
          if (!hit) return null;
          return { model: hit, provider: hit.provider, id: hit.id, canonical: `${hit.provider}/${hit.id}` };
        },
        setModel: async (m) => hasAuth(m),
        getThinkingLevel: () => "high",
        setThinkingLevel() {},
        getSession: () => 1,
        notifyResult: (r) => results.push(r),
        onApplied() {},
        log: silentLog,
      });

      await handle({ rid: "t1", model: "zai/glm-5.2" });
      check("two-tier: a model absent from the catalog but keyed APPLIES",
        results[0].ok === true && results[0].model === "zai/glm-5.2");

      await handle({ rid: "t2", model: "google/gemini-3-pro" });
      check("two-tier: a model that resolves with NO key is refused, not applied",
        results[1].ok === false && results[1].code === "no_api_key");
      check("two-tier: the refusal names the provider so the fix is obvious",
        results[1].detail.includes("google"));

      await handle({ rid: "t3", model: "not-a-model" });
      check("two-tier: nothing resolving is unknown_model, a DIFFERENT code",
        results[2].ok === false && results[2].code === "unknown_model");

      await handle({ rid: "t4", model: "gpt-5.6-sol" });
      check("two-tier: a bare id resolves and is echoed CANONICAL for the row to match",
        results[3].ok === true && results[3].model === "openai-codex/gpt-5.6-sol");
    }
  }

  console.log("");
  if (failures > 0) {
    console.log(`FAILED: ${failures} check(s) failed`);
    process.exit(1);
  }
  console.log("ALL CHECKS PASSED");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
