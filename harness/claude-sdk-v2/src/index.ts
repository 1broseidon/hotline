/**
 * hotline claude-sdk harness (0.2.0 rebuild) — entry point.
 *
 * A managed edition of the claude harness built on the Claude Agent SDK, rebuilt
 * around a turn ledger that guarantees delivery: it knows, for every inbound
 * envelope, which SDK turn consumed it and whether that turn answered the
 * channel. An operator turn that ends in bare assistant text (no reply call) is
 * caught by the M1 fallback lane; a dead credential is contained by M2's auth
 * watch instead of respawning forever in silence.
 *
 * Built clean beside the 0.1 harness; child identity stays `claude-sdk`, so the
 * Go side selects it exactly as before — pointing HOTLINE_CLAUDE_SDK_ENTRY at
 * this dist/index.js IS the swap.
 *
 * Exit codes:
 *   0 clean shutdown (SIGTERM/SIGINT)
 *   1 agent stream ended/failed unexpectedly (supervisor respawns)
 *   3 fatal child condition (binary missing, box ownership conflict)
 *   4 child never became ready within the startup timeout
 *   5 auth-fatal: credentials dead ≥3 times; supervisor cold-loops (M2)
 */

import { spawn } from "node:child_process";
import type { Options, Query, SDKUserMessage } from "@anthropic-ai/claude-agent-sdk";
import { ChildManager } from "@1broseidon/hotline-harness-core/child";
import {
  timeoutsFromEnv,
  mcpCallTool,
  type InitializeResult,
  type McpTool,
  JsonRpcClient,
} from "@1broseidon/hotline-harness-core/jsonrpc";
import { AsyncQueue } from "@1broseidon/hotline-harness-core/queue";
import { sessionFilePath, loadSessionId } from "@1broseidon/hotline-harness-core/session";
import { resolveAuth } from "@1broseidon/hotline-harness-core/auth";
import { toInboundTurn, internalTurn } from "./inbound.js";
import { buildHotlineProxy } from "./proxy.js";
import { runAgent } from "./agent.js";
import { createSdkApplyHandler } from "./sdkapply.js";
import { createJobCards, autoJobsEnabled } from "./jobcards.js";
import { buildSdkCatalog } from "./catalog.js";
import { SdkMissionLoop, parseContextCap, usageFromSdk } from "./missionControl.js";
import { TurnLedger } from "./ledger.js";
import { createFallbackExecutor } from "./fallback.js";
import {
  createAuthWatch,
  classifyAuthMessage,
  classifyAuthThrow,
  saveLastOperator,
  loadLastOperator,
} from "./authwatch.js";
import { CONTRACT_REMINDER } from "./contract.js";
import { log } from "./log.js";

const READY_TIMEOUT_MS = 30_000;
const SHUTDOWN_GRACE_MS = 10_000;
const HARNESS_CATALOG_NOTIFICATION = "notifications/hotline/harness_catalog";

/** HOTLINE_SDK_FALLBACK, default ON; disabled by 0|false|off|no (the parser
 * family autoJobsEnabled uses, jobcards.ts). */
function fallbackKnobEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  const v = (env.HOTLINE_SDK_FALLBACK ?? "").trim().toLowerCase();
  return v !== "0" && v !== "false" && v !== "off" && v !== "no";
}

/** Attribution mode (design §3.5): uuid echo by default; pull-window is the
 * conservative fallback if a runtime smoke shows the echo drops the client
 * uuid. Selectable so M1 can ship either way; the boot log states which is
 * live. */
function attributionMode(env: NodeJS.ProcessEnv = process.env): "echo" | "pull-window" {
  return (env.HOTLINE_SDK_ATTRIBUTION ?? "").trim().toLowerCase() === "pull-window"
    ? "pull-window"
    : "echo";
}

/** M1.1 enforcement mode. HOTLINE_SDK_ENFORCE=stop-hook (default) installs a
 * Stop hook that blocks a turn from ending while it still owes the operator a
 * reply — the locked door in front of the fallback lane's safety net. `off`
 * removes the hook (fallback lane only). Independent of HOTLINE_SDK_FALLBACK. */
function enforceMode(env: NodeJS.ProcessEnv = process.env): "stop-hook" | "off" {
  return (env.HOTLINE_SDK_ENFORCE ?? "").trim().toLowerCase() === "off" ? "off" : "stop-hook";
}

/** Max Stop-hook blocks per turn before letting it end and delivering via the
 * fallback lane (design §M1.1). */
const MAX_STOP_BLOCKS = 2;

async function main(): Promise<void> {
  const auth = resolveAuth(process.env);
  log.info(`auth source: ${auth.source} — ${auth.note}`);

  const supervisorDir = process.env.HOTLINE_SUPERVISOR_DIR || undefined;
  const timeouts = timeoutsFromEnv("HOTLINE_SDK");
  const fallbackEnabled = fallbackKnobEnabled();
  const attribution = attributionMode();
  const enforce = enforceMode();

  const queue = new AsyncQueue<SDKUserMessage>();
  let toolsSnapshot: McpTool[] = [];
  let shuttingDown = false;

  // The turn ledger (design §3.4): the single source of truth for delivery
  // attribution, driven from the stream loop + hooks.
  const ledger = new TurnLedger();

  // Whether the box is multi-provider — the reply schema only carries a
  // required `source` property then (server.go withSourceProperty). We read the
  // live reply tool schema rather than guess, so the fallback and the auth
  // notify pass `source` only when the wire actually needs it.
  const multiProvider = (): boolean => {
    const reply = toolsSnapshot.find((t) => t.name === "reply");
    const schema = reply?.inputSchema as { required?: unknown } | undefined;
    return Array.isArray(schema?.required) && (schema!.required as unknown[]).includes("source");
  };

  let resolvedModel: string | undefined;
  let modelCleared = false;
  let effortCleared = false;
  let laneFired = 0;
  const sendHarnessInfo = (): void => {
    const client = manager.getClient();
    if (!client) return;
    const effort = process.env.HOTLINE_SDK_EFFORT || undefined;
    client.notify("notifications/hotline/harness_info", {
      harness: "claude-sdk",
      model: resolvedModel ?? (modelCleared ? "" : undefined),
      effort: effort ?? (effortCleared ? "" : undefined),
      // Lane telemetry (design §3.7): the option-c decision input. Presence
      // only when >0; old binaries unmarshal-ignore the unknown field.
      fallback_count: laneFired > 0 ? laneFired : undefined,
    });
  };

  let readyResolve: ((init: InitializeResult) => void) | null = null;
  const firstReady = new Promise<InitializeResult>((resolve) => {
    readyResolve = resolve;
  });

  let activeQuery: Query | null = null;
  let sessionGeneration = 0;
  const handleSdkApply = createSdkApplyHandler({
    getSession: () =>
      activeQuery === null ? null : { query: activeQuery, generation: sessionGeneration },
    notifyResult: (result) => {
      const client = manager.getClient();
      if (!client) {
        log.warn(`sdk_apply ${result.rid}: child down; result dropped`);
        return;
      }
      client.notify("notifications/hotline/sdk_apply_result", result);
    },
    onApplied: (applied) => {
      if (applied.model !== undefined) {
        if (applied.model === "") delete process.env.HOTLINE_SDK_MODEL;
        else process.env.HOTLINE_SDK_MODEL = applied.model;
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

  const sendHarnessCatalog = (): void => {
    const client = manager.getClient();
    const q = activeQuery;
    if (!client || !q) return;
    q.supportedModels()
      .then((models) => {
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

  const jobCards = autoJobsEnabled()
    ? createJobCards({ binary: process.env.HOTLINE_BIN || "hotline", log })
    : null;

  // M1 fallback lane executor: sends the buffered turn text via reply on the
  // live child when an operator turn ended uncovered.
  const fallbackExecutor = createFallbackExecutor({
    getClient: () => manager.getClient(),
    multiProvider,
    log,
    onFired: () => {
      laneFired++;
      sendHarnessInfo();
    },
  });

  // M2 auth watch: the notify closure reads last-operator and speaks through the
  // still-healthy channel child.
  const authWatch = createAuthWatch({
    supervisorDir,
    log,
    notify: async (errorText: string): Promise<boolean> => {
      const client = manager.getClient();
      const last = loadLastOperator(supervisorDir);
      if (!client || !last) return false;
      const args: Record<string, unknown> = {
        chat_id: last.chat_id,
        text: `my Claude credentials are dead (${errorText}) — I can't think until you run \`claude login\` on the box / fix the key. I'll keep checking every 10 minutes.`,
      };
      if (last.source && multiProvider()) args.source = last.source;
      try {
        const res = await mcpCallTool(client, "reply", args, timeouts.callTimeoutMs);
        return res.isError !== true;
      } catch {
        return false;
      }
    },
  });

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
    // Mission-injected turns are uuid-stamped and marked internal in the ledger
    // so the settle procedure excludes them (no monologue leakage) while still
    // feeding mission fencing.
    sendHandoffTurn: (prompt) => {
      const { msg, uuid } = internalTurn(prompt);
      ledger.register(uuid, null, "handoff");
      queue.push(msg);
    },
    queueCompact: (instructions) => {
      const { msg, uuid } = internalTurn(`/compact ${instructions}`);
      ledger.register(uuid, null, "compact");
      queue.push(msg);
    },
    mechanicalHandoff: (state, next) => runMechanicalHandoff(state, next),
    log: (msg) => log.info(`mission: ${msg}`),
    warn: (msg) => log.warn(`mission: ${msg}`),
  });
  log.info(`mission: handoff loop armed (cap ${missionCap ?? "unset"})`);
  log.info(`lane: attribution=${attribution} fallback=${fallbackEnabled ? "on" : "off"} enforce=${enforce}`);

  const missionHooks: NonNullable<Options["hooks"]> = {
    // Layer 1: ALWAYS re-inject the reply contract as additionalContext (design
    // §2.3 — the per-turn recency the one-shot preset append lacked), plus the
    // armed 80% nudge when present. Also capture the last REAL user prompt for
    // the mechanical fallback's state note.
    UserPromptSubmit: [
      {
        hooks: [
          async (input) => {
            const prompt = (input as { prompt?: string }).prompt ?? "";
            if (prompt && !prompt.startsWith("/compact") && !prompt.startsWith("[mission-control]")) {
              missionLoop.noteUserPrompt(prompt);
            }
            let additionalContext = CONTRACT_REMINDER;
            if (pendingNudge) {
              additionalContext = `${CONTRACT_REMINDER}\n\n${pendingNudge}`;
              pendingNudge = "";
            }
            return {
              hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext },
            };
          },
        ],
      },
    ],
    PostToolUse: [
      // Mission handoff observation.
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
      // M1 T5: a reply that SUCCEEDED covers its (source, chat_id) group. A
      // reply the proxy answered with the retryable channel-down isError covers
      // nothing (success-only, design §3.4 T5).
      {
        matcher: "mcp__hotline__reply",
        hooks: [
          async (input) => {
            const i = input as {
              tool_input?: { source?: unknown; chat_id?: unknown };
              tool_response?: { isError?: unknown };
            };
            if (i.tool_response?.isError === true) return { continue: true };
            const source = typeof i.tool_input?.source === "string" ? i.tool_input.source : undefined;
            const chatId = typeof i.tool_input?.chat_id === "string" ? i.tool_input.chat_id : undefined;
            ledger.onReplySuccess(source, chatId);
            return { continue: true };
          },
        ],
      },
    ],
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

  // M1.1: the locked door. When the ending turn still owes the operator a reply
  // (an operator conversation is awaiting delivery and this turn wasn't covered
  // by a successful reply call), block the stop and instruct an immediate reply
  // in voice — the same `activeUncoveredGroups()` predicate the settle uses, so
  // the door and the fallback net agree on "unanswered". Cap: MAX_STOP_BLOCKS
  // per turn (epoch-scoped), then let it end and the fallback lane delivers.
  if (enforce === "stop-hook") {
    missionHooks.Stop = [
      {
        hooks: [
          async () => {
            try {
              const pending = ledger.activeUncoveredGroups();
              if (pending.length === 0) return { continue: true };
              const chat = pending[0].chat_id;
              if (ledger.stopBlockCount >= MAX_STOP_BLOCKS) {
                log.info(
                  `lane: settle blocked-by-hook exhausted (chat=${chat} blocks=${ledger.stopBlockCount}) — letting the turn end; fallback lane delivers`,
                );
                return { continue: true };
              }
              ledger.noteStopBlock();
              log.info(
                `stop-hook: blocked-by-hook (${ledger.stopBlockCount}/${MAX_STOP_BLOCKS} chat=${chat}) — operator turn ended without a reply call`,
              );
              return {
                decision: "block",
                reason:
                  `You ended this turn without calling mcp__hotline__reply, so the operator has NOT seen anything you wrote — you are running headless, there is no terminal, your assistant text goes nowhere. Call mcp__hotline__reply now (chat_id="${chat}") with your answer, in your own voice, in bubbles. That tool is the only way to be heard.`,
              };
            } catch (err) {
              log.warn(`stop-hook: enforcement threw: ${String(err)}`);
              return { continue: true };
            }
          },
        ],
      },
    ];
  }

  const manager = new ChildManager({
    binary: process.env.HOTLINE_BIN || "hotline",
    args: ["run"],
    env: { ...process.env, HOTLINE_HARNESS: "claude-sdk" },
    clientName: "hotline-claude-sdk",
    timeouts: timeoutsFromEnv("HOTLINE_SDK"),
    installHint: "See the harness/claude-sdk-v2 README.",
    log,
    onNotification: (method, params) => {
      if (method === "notifications/hotline/sdk_apply") {
        void handleSdkApply(params);
        return;
      }
      if (method !== "notifications/claude/channel") {
        log.info(`ignoring notification ${method}`);
        return;
      }
      const turn = toInboundTurn(params);
      if (turn === null) {
        log.warn("dropping channel notification with no usable content");
        return;
      }
      // T1: register with the ledger BEFORE the push, so the attribution key
      // exists before the SDK can echo it.
      const klass = ledger.register(turn.uuid, turn.env);
      if (klass === "operator" && turn.env?.chat_id) {
        // M2 last-operator target (design §4.2): persisted on every operator
        // envelope the ledger registers.
        saveLastOperator(supervisorDir, turn.env.source, turn.env.chat_id);
      }
      queue.push(turn.msg);
    },
    onReady: (_client: JsonRpcClient, init: InitializeResult, tools: McpTool[]) => {
      toolsSnapshot = tools;
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

  const sessionFile = sessionFilePath("claude-sdk-session.json");
  const savedSessionId = loadSessionId(sessionFile);
  if (savedSessionId) log.info(`resuming SDK session ${savedSessionId}`);

  const proxy = buildHotlineProxy({
    getClient: () => manager.getClient(),
    getTools: () => toolsSnapshot,
  });

  const abortController = new AbortController();

  const shutdown = (sig: string): void => {
    if (shuttingDown) return;
    shuttingDown = true;
    log.info(`${sig} received; shutting down`);
    const unsettled = ledger.unsettledCount();
    if (unsettled > 0) log.warn(`lane: ${unsettled} envelope(s) unsettled at shutdown (no fallback on the abort path)`);
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
      modelCleared = false;
      sendHarnessInfo();
      sendHarnessCatalog();
    },
    onQuery: (q) => {
      activeQuery = q;
      sessionGeneration += 1;
    },
    onMessage: jobCards ? (msg) => jobCards.onMessage(msg) : undefined,
    hooks: missionHooks,
    // M1 ledger stream-side transitions.
    onUserEcho: attribution === "echo" ? (uuid, isReplay) => ledger.onUserEcho(uuid, isReplay) : undefined,
    onPull: attribution === "pull-window" ? (uuid) => ledger.onUserEcho(uuid, false) : undefined,
    onAssistantText: (texts) => ledger.onAssistantText(texts),
    onSettle: async (subtype) => {
      const plan = ledger.onResult(subtype, fallbackEnabled);
      // Goal 3: one log line per settle outcome — tonight's silent-miss
      // debugging must never repeat. (fired is logged by the executor below;
      // blocked-by-hook is logged by the Stop hook.)
      if (plan.ambiguous) {
        log.info(`lane: settle ambiguous-continuation (>1 conversation awaiting, none delivered this turn) — forwarding nothing`);
      }
      if (plan.multiTarget) {
        log.info(`lane: lane_multi_target (groups settled together this turn)`);
      }
      for (const g of plan.replySatisfied) {
        log.info(`lane: settle reply-satisfied (source=${g.source ?? "-"} chat=${g.chat_id})`);
      }
      for (const m of plan.misses) {
        log.info(`lane: settle miss (source=${m.source ?? "-"} chat=${m.chat_id} why=${m.reason})`);
      }
      if (plan.fallbacks.length > 0) await fallbackExecutor.run(plan.fallbacks);
    },
    onTeardown: () => ledger.revertOnTeardown(),
    // M2 auth containment.
    classifyAuthFailure: classifyAuthMessage,
    classifyAuthThrow: classifyAuthThrow,
    onAuthReset: () => authWatch.onInit(),
    onAuthFailure: (errorText) => authWatch.onAuthFailure(errorText),
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
