/**
 * hotline claude-sdk harness — entry point.
 *
 * An alternate managed edition of the claude harness built on the Claude Agent
 * SDK. Shaped like the pi harness: a headless piped process under `hotline up`
 * that spawns one `hotline run` child (HOTLINE_HARNESS=claude-sdk — the Go
 * side's first-class injected-harness branch, run_claudesdk.go), injects
 * inbound channel notifications as user turns into a long-lived SDK session,
 * and proxies hotline's tools to the agent via an in-process MCP server. This
 * owns the two-way loop in our own code — no
 * --dangerously-load-development-channels, no pty, no consent UI.
 *
 * Exit codes:
 *   0 clean shutdown (SIGTERM/SIGINT)
 *   1 agent stream ended/failed unexpectedly (supervisor respawns)
 *   3 fatal child condition (binary missing, box ownership conflict)
 *   4 child never became ready within the startup timeout
 */

import { spawn } from "node:child_process";
import type { Options, Query, SDKUserMessage } from "@anthropic-ai/claude-agent-sdk";
import { ChildManager } from "@1broseidon/hotline-harness-core/child";
import {
  timeoutsFromEnv,
  type InitializeResult,
  type McpTool,
  JsonRpcClient,
} from "@1broseidon/hotline-harness-core/jsonrpc";
import { AsyncQueue } from "@1broseidon/hotline-harness-core/queue";
import { sessionFilePath, loadSessionId } from "@1broseidon/hotline-harness-core/session";
import { resolveAuth } from "@1broseidon/hotline-harness-core/auth";
import { toUserMessage } from "./inbound.js";
import { buildHotlineProxy } from "./proxy.js";
import { runAgent } from "./agent.js";
import { createSdkApplyHandler } from "./sdkapply.js";
import { createJobCards, autoJobsEnabled } from "./jobcards.js";
import { buildSdkCatalog } from "./catalog.js";
import { SdkMissionLoop, parseContextCap, usageFromSdk } from "./missionControl.js";
import { log } from "./log.js";

const READY_TIMEOUT_MS = 30_000;
const SHUTDOWN_GRACE_MS = 10_000;
const HARNESS_CATALOG_NOTIFICATION = "notifications/hotline/harness_catalog";

/** Wrap a plain string as a streaming-input user turn (same shape as
 * inbound.ts's toUserMessage output), for mission-control injected turns. */
function sdkUserTurn(content: string): SDKUserMessage {
  return { type: "user", message: { role: "user", content }, parent_tool_use_id: null };
}

async function main(): Promise<void> {
  // 1. Auth report (never the value): the SDK resolves credentials itself; we
  //    log which source wins so harness.log tells the truth. No env var set is
  //    NOT an error — a stored `claude login` may exist.
  const auth = resolveAuth(process.env);
  log.info(`auth source: ${auth.source} — ${auth.note}`);

  // 2. Start the hotline child and await its first ready (handshake + tools).
  const queue = new AsyncQueue<SDKUserMessage>();
  let toolsSnapshot: McpTool[] = [];
  let shuttingDown = false;

  // Wire metadata (spec §5.3): report harness kind + resolved model to the
  // run child over the same stdio, so the box can stamp welcome/agent_state.
  // The harness field is ALWAYS sent (the box needs no TS-version inference);
  // the model rides along once the SDK init resolves it. Resent on every
  // child ready — a respawned child starts with only its config seed.
  //
  // PRESENCE, not just value (hot-clear amendment). The box distinguishes three
  // states per field: omitted = "not reported, leave it alone"; "" = "reported
  // as CLEARED"; a value = the live one. Omitting a cleared field is what used
  // to make the box keep advertising the old model after a clear — and keep its
  // no-op check believing that model was live, so re-selecting it later was
  // answered "already effective" and never applied.
  //
  // The `cleared` flags are only set by an explicit hot clear. At boot an unset
  // knob is genuinely UNKNOWN, not cleared, so it stays omitted and the box's
  // config seed survives — which is the pre-amendment behaviour, unchanged.
  let resolvedModel: string | undefined;
  let modelCleared = false;
  let effortCleared = false;
  const sendHarnessInfo = (): void => {
    const client = manager.getClient();
    if (!client) return; // child down mid-respawn: dropped, resent on next ready
    const effort = process.env.HOTLINE_SDK_EFFORT || undefined;
    client.notify("notifications/hotline/harness_info", {
      harness: "claude-sdk",
      model: resolvedModel ?? (modelCleared ? "" : undefined),
      effort: effort ?? (effortCleared ? "" : undefined),
    });
  };

  let readyResolve: ((init: InitializeResult) => void) | null = null;
  const firstReady = new Promise<InitializeResult>((resolve) => {
    readyResolve = resolve;
  });

  // SDK hot-model apply (amendment 2026-07-19): the live Query handle, set by
  // runAgent's onQuery hook (null between sessions). The box forwards a
  // model-only set_sdk_config as a sdk_apply notification; the handler
  // validates against the CLI's supportedModels, calls the setModel control
  // on the live session (no restart, the wire never drops), restamps
  // harness_info, and answers on sdk_apply_result — the box persists to .env
  // only after that ok (persist-after-ok).
  //
  // sessionGeneration fences an in-flight apply against the session moving
  // underneath it. An apply is several awaits long (supportedModels, then the
  // control calls), and both things it depends on can be replaced mid-flight:
  // runAgent ends one query and starts another (a stream end, or the
  // retry-without-resume path), and ChildManager respawns the hotline child
  // that carries the result notification. Bumping here on BOTH events lets
  // sdkapply re-check after every await and refuse (no_session) rather than
  // push a model onto a dead query and report success for the live one.
  let activeQuery: Query | null = null;
  let sessionGeneration = 0;
  const handleSdkApply = createSdkApplyHandler({
    getSession: () =>
      activeQuery === null ? null : { query: activeQuery, generation: sessionGeneration },
    notifyResult: (result) => {
      const client = manager.getClient();
      if (!client) {
        // Child down mid-respawn: the box's pending timer surfaces
        // harness_unreachable; nothing was persisted, retry is safe.
        log.warn(`sdk_apply ${result.rid}: child down; result dropped`);
        return;
      }
      client.notify("notifications/hotline/sdk_apply_result", result);
    },
    onApplied: (applied) => {
      // The harness env is what a harness-spawned respawned child would
      // inherit, and buildOptions re-reads it on a same-process re-query — a
      // respawn must not revert a confirmed hot apply even before the box's
      // .env persist lands. Only the fields the request changed are touched.
      if (applied.model !== undefined) {
        if (applied.model === "") delete process.env.HOTLINE_SDK_MODEL;
        else process.env.HOTLINE_SDK_MODEL = applied.model;
        // Clear-to-default: after setModel(undefined) no init message fires and
        // there is no getModel, so the id the default resolves to is unknowable
        // here. That is why the clear is reported as PRESENCE ("") rather than
        // by omission — the box learns "there is no explicit model any more",
        // which is true and useful, instead of nothing, which left it
        // advertising the old id indefinitely. The next init message replaces
        // it with the resolved truth.
        resolvedModel = applied.resolvedModel;
        modelCleared = applied.model === "";
      }
      if (applied.effort !== undefined) {
        if (applied.effort === "") delete process.env.HOTLINE_SDK_EFFORT;
        else process.env.HOTLINE_SDK_EFFORT = applied.effort;
        effortCleared = applied.effort === "";
      }
      sendHarnessInfo();
    },
    log,
  });

  // Model catalog (spec workstream 3): the selectable set behind harness_info,
  // so the app's model row shows THIS box's models rather than a compiled-in
  // list. pi precedent: sent on child-ready. claude-sdk has a second edge — a
  // live Query is required for supportedModels(), and the query is not up yet
  // at the FIRST child-ready — so this no-ops unless BOTH the child and an
  // active query exist, and is called again from onInit (once per query,
  // right after the init message — same moment resolvedModel restamps
  // harness_info). Fire-and-forget, pi posture: never throws, empty list = box
  // keeps its curated fallback.
  const sendHarnessCatalog = (): void => {
    const client = manager.getClient();
    const q = activeQuery;
    if (!client || !q) return;
    q.supportedModels()
      .then((models) => {
        // The client may have gone down mid-fetch (respawn); the notify
        // guard below re-checks, same as pi's report-dropped path.
        const currentClient = manager.getClient();
        if (!currentClient) {
          log.warn("catalog: child down; report dropped");
          return;
        }
        const catalog = buildSdkCatalog(models);
        currentClient.notify(HARNESS_CATALOG_NOTIFICATION, catalog);
        log.info(
          `catalog: ${catalog.models.length} model(s) from ${catalog.source}` +
            `${catalog.truncated ? " (truncated)" : ""}`,
        );
      })
      .catch((err: unknown) => log.warn(`catalog: report failed: ${String(err)}`));
  };

  // FB13 job cards (spec workstream 1): bridge task_started/task_updated
  // stream messages to `hotline job`. Opt-out via HOTLINE_AUTO_JOBS; the
  // binary is HOTLINE_BIN (the supervisor's own binary, pinned into the
  // child env at cmd_up.go:443-444) falling back to "hotline" on PATH.
  const jobCards = autoJobsEnabled()
    ? createJobCards({ binary: process.env.HOTLINE_BIN || "hotline", log })
    : null;

  // Mission control (spec workstream 2): the compaction/handoff loop. Reads
  // HOTLINE_MC_CONTEXT_CAP from real env (D2: a .env-only cap reaches neither
  // this harness nor pi — real-env-only, deliberately identical to pi). The
  // loop is a pure state machine; these closures bridge it to the live session:
  //   - armNudge  → pendingNudge, drained once by the UserPromptSubmit hook.
  //   - sendHandoffTurn / queueCompact → queue.push, the SAME inbound path
  //     operator turns take, so ordering with operator traffic is preserved.
  //   - mechanicalHandoff → `hotline mission handoff --trigger auto …`; a
  //     promise so the PreCompact hook can await it before compaction proceeds.
  const hotlineBinary = process.env.HOTLINE_BIN || "hotline";
  const missionCap = parseContextCap(process.env);
  let pendingNudge = "";
  const runMechanicalHandoff = (state: string, next: string): Promise<void> =>
    new Promise((resolve) => {
      try {
        const child = spawn(
          hotlineBinary,
          ["mission", "handoff", "--trigger", "auto", "--state", state, "--next", next],
          { stdio: "ignore" },
        );
        child.on("error", (err: unknown) => {
          log.warn(`mission: mechanical handoff failed: ${String(err)}`);
          resolve();
        });
        child.on("close", () => resolve());
      } catch (err) {
        log.warn(`mission: mechanical handoff threw: ${String(err)}`);
        resolve();
      }
    });
  const missionLoop = new SdkMissionLoop(missionCap, {
    armNudge: (line) => {
      pendingNudge = line;
    },
    sendHandoffTurn: (prompt) => queue.push(sdkUserTurn(prompt)),
    queueCompact: (instructions) => queue.push(sdkUserTurn(`/compact ${instructions}`)),
    mechanicalHandoff: (state, next) => runMechanicalHandoff(state, next),
    log: (msg) => log.info(`mission: ${msg}`),
    warn: (msg) => log.warn(`mission: ${msg}`),
  });
  log.info(`mission: handoff loop armed (cap ${missionCap ?? "unset"})`);

  // The hooks half of mission control. Registered on Options.hooks; every
  // callback is best-effort and never throws a session down.
  const missionHooks: NonNullable<Options["hooks"]> = {
    // Layer 1: inject the armed 80% nudge as additionalContext, consumed once.
    // Also capture the last REAL user prompt for the mechanical fallback's
    // state note (skipping our own injected turns so it stays meaningful).
    UserPromptSubmit: [
      {
        hooks: [
          async (input) => {
            const prompt = (input as { prompt?: string }).prompt ?? "";
            if (prompt && !prompt.startsWith("/compact") && !prompt.startsWith("[mission-control]")) {
              missionLoop.noteUserPrompt(prompt);
            }
            if (pendingNudge) {
              const additionalContext = pendingNudge;
              pendingNudge = "";
              return {
                hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext },
              };
            }
            return { continue: true };
          },
        ],
      },
    ],
    // The mission-handoff observation (equal to pi's tool_result seam): the
    // result's error status is visible here.
    PostToolUse: [
      {
        matcher: "mcp__hotline__mission",
        hooks: [
          async (input) => {
            const i = input as {
              tool_input?: { action?: unknown };
              tool_response?: { isError?: unknown };
            };
            const isError = i.tool_response?.isError === true;
            if (!isError && i.tool_input?.action === "handoff") missionLoop.noteHandoffWritten();
            return { continue: true };
          },
        ],
      },
    ],
    // The CLI-auto divergence (spec §2 resolution): PreCompact cannot cancel,
    // but it is AWAITED — so on an automatic compaction with no fresh handoff we
    // write the mechanical handoff synchronously and compaction proceeds only
    // after it lands. A manual `/compact` (including our own cap-path one) is
    // left alone.
    PreCompact: [
      {
        hooks: [
          async (input) => {
            const trigger = (input as { trigger?: "manual" | "auto" }).trigger ?? "auto";
            try {
              const { mechanical } = missionLoop.onPreCompact(trigger);
              if (mechanical) {
                log.info(`mission: PreCompact(${trigger}) with no fresh handoff; writing mechanical handoff first`);
                await runMechanicalHandoff(mechanical.state, mechanical.next);
              }
            } catch (err) {
              log.warn(`mission: PreCompact failed: ${String(err)}`);
            }
            return { continue: true };
          },
        ],
      },
    ],
  };

  const manager = new ChildManager({
    binary: process.env.HOTLINE_BIN || "hotline",
    args: ["run"],
    // The Go child selects its injected-harness branch (run_claudesdk.go:
    // pre-rendered <channel> envelope, permission relay off, uncapped
    // instructions, replayCatchup) from HOTLINE_HARNESS. Set it explicitly —
    // not merely inherited — so a stale value in an operator shell can never
    // flip the child's identity; the child's ClaimBox must match the
    // supervisor's claude-sdk reservation. HOTLINE_SUPERVISOR_DIR and the
    // owner lease flow through untouched.
    env: { ...process.env, HOTLINE_HARNESS: "claude-sdk" },
    clientName: "hotline-claude-sdk",
    timeouts: timeoutsFromEnv("HOTLINE_SDK"),
    installHint: "See the harness/claude-sdk README.",
    log,
    onNotification: (method, params) => {
      if (method === "notifications/hotline/sdk_apply") {
        // Hot model apply: async, never blocks the notification dispatcher;
        // every path settles by notifying an sdk_apply_result.
        void handleSdkApply(params);
        return;
      }
      if (method !== "notifications/claude/channel") {
        log.info(`ignoring notification ${method}`);
        return;
      }
      const msg = toUserMessage(params);
      if (msg === null) {
        log.warn("dropping channel notification with no usable content");
        return;
      }
      queue.push(msg);
    },
    onReady: (_client: JsonRpcClient, init: InitializeResult, tools: McpTool[]) => {
      toolsSnapshot = tools;
      // A respawned child is a new result channel: any apply captured against
      // the old one can no longer be answered, so fence it.
      sessionGeneration += 1;
      sendHarnessInfo();
      sendHarnessCatalog();
      if (readyResolve) {
        const r = readyResolve;
        readyResolve = null;
        r(init);
      }
    },
    onDown: (reason) => log.warn(`hotline child down: ${reason}`),
    onFatal: (message) => {
      log.error(`fatal: ${message}`);
      process.exit(3);
    },
  });
  manager.start();

  const init = await Promise.race([
    firstReady,
    new Promise<never>((_, reject) =>
      setTimeout(() => reject(new Error(`hotline child not ready within ${READY_TIMEOUT_MS}ms`)), READY_TIMEOUT_MS).unref(),
    ),
  ]).catch((err: Error) => {
    log.error(err.message);
    manager.stop();
    process.exit(4);
  });

  const instructions = init.instructions ?? "";
  if (instructions === "") log.warn("child initialize carried no instructions");

  // 3. Session continuity.
  const sessionFile = sessionFilePath("claude-sdk-session.json");
  const savedSessionId = loadSessionId(sessionFile);
  if (savedSessionId) log.info(`resuming SDK session ${savedSessionId}`);

  // 4. The SDK session, fed by the inbound queue.
  const proxy = buildHotlineProxy({
    getClient: () => manager.getClient(),
    getTools: () => toolsSnapshot,
  });

  const abortController = new AbortController();

  const shutdown = (sig: string): void => {
    if (shuttingDown) return;
    shuttingDown = true;
    log.info(`${sig} received; shutting down`);
    // Close the generator; the SDK finishes the in-flight turn then ends the
    // stream. A grace timer aborts a wedged turn.
    queue.close();
    setTimeout(() => {
      log.warn(`shutdown grace period (${SHUTDOWN_GRACE_MS}ms) expired; aborting session`);
      abortController.abort();
    }, SHUTDOWN_GRACE_MS).unref();
  };
  process.once("SIGTERM", () => shutdown("SIGTERM"));
  process.once("SIGINT", () => shutdown("SIGINT"));

  const code = await runAgent({
    queue,
    proxy,
    instructions,
    sessionFile,
    savedSessionId,
    abortController,
    isShuttingDown: () => shuttingDown,
    onInit: (model) => {
      resolvedModel = model;
      modelCleared = false; // the session resolved a real id; nothing is cleared
      sendHarnessInfo();
      // The second catalog edge (spec workstream 3): a live Query exists now,
      // right after the init message — the earliest supportedModels() can run.
      sendHarnessCatalog();
    },
    onQuery: (q) => {
      activeQuery = q;
      sessionGeneration += 1;
    },
    onMessage: jobCards ? (msg) => jobCards.onMessage(msg) : undefined,
    // Mission control (spec workstream 2): hooks ride Options; onResult polls
    // getContextUsage() per settle and steps the loop; onCompactBoundary resets
    // the cycle once compaction lands.
    hooks: missionHooks,
    onResult: async (q) => {
      try {
        const res = await q.getContextUsage();
        missionLoop.onResult(usageFromSdk(res));
      } catch (err) {
        log.warn(`mission: context usage read failed: ${String(err)}`);
      }
    },
    onCompactBoundary: (trigger) => {
      log.info(`mission: compaction boundary (${trigger}); resetting cycle`);
      missionLoop.onCompactBoundary();
    },
  });

  jobCards?.dispose();
  manager.stop();
  process.exit(code);
}

main().catch((err: unknown) => {
  log.error(`unhandled: ${err instanceof Error ? (err.stack ?? err.message) : String(err)}`);
  process.exit(1);
});
