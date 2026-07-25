/**
 * @1broseidon/hotline-pi — the hotline channel extension for Pi.
 *
 * Pi is one of hotline's four harnesses (next to Claude Code, claude-sdk, and
 * OpenCode).
 * This extension spawns `hotline run` as a child, bridges its stdio JSON-RPC
 * surface into the Pi session, and exposes hotline's tools (reply, react, …) as
 * first-class Pi tools. Inbound Telegram/Discord/Signal messages arrive as
 * `notifications/claude/channel` and are injected as user turns via
 * pi.sendUserMessage. No MCP is visible anywhere on the Pi side.
 *
 * The child spawn/respawn plumbing and the hand-rolled JSONL client live in
 * @1broseidon/hotline-harness-core (shared with the claude-sdk harness);
 * this extension binds them with pi identity: clientInfo "hotline-pi",
 * HOTLINE_PI_* timeout envs, HOTLINE_PI_LOG file sink.
 */

import {
  getAgentDir,
  resolveCliModel,
  resolveModelScopeWithDiagnostics,
  SettingsManager,
  type ExtensionAPI,
  type ExtensionContext,
} from "@earendil-works/pi-coding-agent";
import { ChildManager } from "@1broseidon/hotline-harness-core/child";
import {
  mcpCallTool,
  timeoutsFromEnv,
  type InitializeResult,
  type McpTool,
} from "@1broseidon/hotline-harness-core/jsonrpc";
import { createLog } from "@1broseidon/hotline-harness-core/log";
import { deliverToAgent as deliverInbound } from "./deliver.js";
import { wireJobCards } from "./jobcards.js";
import {
  createPiApplyHandler,
  IdentityAnnouncer,
  type PiResolvedModel,
  type PiThinkingLevel,
} from "./piapply.js";
import { buildCatalog, parsePiModels, type PiModelLike } from "./modelcatalog.js";

import {
  clampContextCap,
  compactWithCallbacks,
  COMPACTION_CAP_MARGIN,
  MissionControlLoop,
  parseContextCap,
  type LoopActions,
} from "./missionControl.js";

const log = createLog("hotline-pi", "HOTLINE_PI_LOG");

/** FB13: whether auto job-card reporting is enabled (opt-out via HOTLINE_AUTO_JOBS=0/false). */
function autoJobsEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  const v = (env.HOTLINE_AUTO_JOBS ?? "").trim().toLowerCase();
  return v !== "0" && v !== "false" && v !== "off" && v !== "no";
}

const CHANNEL_NOTIFICATION = "notifications/claude/channel";
const HARNESS_INFO_NOTIFICATION = "notifications/hotline/harness_info";
const SDK_APPLY_NOTIFICATION = "notifications/hotline/sdk_apply";
const SDK_APPLY_RESULT_NOTIFICATION = "notifications/hotline/sdk_apply_result";
const HARNESS_CATALOG_NOTIFICATION = "notifications/hotline/harness_catalog";
const PI_DEFAULT_KEEP_RECENT_TOKENS = 20_000;

/**
 * ExtensionContext does not expose pi's live SettingsManager (0.80.6). Rebuild
 * the same public settings view the standard CLI constructs from cwd, agent dir,
 * and project trust. This is exact for hotline's CLI-hosted pi sessions; the
 * fallback protects SDK/custom hosts whose in-memory overrides are not visible.
 */
function readPiKeepRecentTokens(sessionCtx: ExtensionContext): number {
  try {
    const settings = SettingsManager.create(sessionCtx.cwd, getAgentDir(), {
      projectTrusted: sessionCtx.isProjectTrusted(),
    });
    const keepRecent = settings.getCompactionKeepRecentTokens();
    for (const issue of settings.drainErrors()) {
      log.warn(`mission: pi compaction settings read failed (${issue.scope}): ${String(issue.error)}`);
    }
    return keepRecent;
  } catch (err) {
    log.warn(
      `mission: could not read pi keepRecentTokens; using default ${PI_DEFAULT_KEEP_RECENT_TOKENS}: ${String(err)}`,
    );
    return PI_DEFAULT_KEEP_RECENT_TOKENS;
  }
}

/**
 * pi's own scope fallback: `settingsManager.getEnabledModels()`, the list
 * main.js uses when no --models flag was passed. Rebuilt from cwd + agent dir +
 * project trust exactly like readPiKeepRecentTokens, and for the same reason —
 * ExtensionContext does not hand out the live SettingsManager (0.80.6). Any
 * failure degrades to "no setting", which lands on the available-models branch:
 * a wider menu than the truth, never a narrower one.
 */
function readPiEnabledModels(sessionCtx: ExtensionContext): string[] | undefined {
  try {
    const settings = SettingsManager.create(sessionCtx.cwd, getAgentDir(), {
      projectTrusted: sessionCtx.isProjectTrusted(),
    });
    const enabled = settings.getEnabledModels();
    for (const issue of settings.drainErrors()) {
      log.warn(`catalog: pi settings read failed (${issue.scope}): ${String(issue.error)}`);
    }
    return enabled;
  } catch (err) {
    log.warn(`catalog: could not read pi enabledModels: ${String(err)}`);
    return undefined;
  }
}

/** First sentence of a description, for the tool's one-line promptSnippet. */
function firstSentence(desc: string): string {
  const trimmed = desc.trim();
  const dot = trimmed.indexOf(". ");
  return dot > 0 ? trimmed.slice(0, dot + 1) : trimmed;
}

export default function hotlinePi(pi: ExtensionAPI): void {
  // Async delegation is provided by a separately-installed plugin (its `Agent`
  // tool), not this package. This extension is just the channel bridge: it
  // spawns `hotline run`, registers hotline's tools, and injects inbound
  // messages.
  let manager: ChildManager | null = null;
  // ONE resolved timeout family for this harness, shared by the child manager
  // and by every tool call. The core extraction left the tool-call site without
  // it, so mcpCallTool silently fell back to core's 60s default and
  // HOTLINE_PI_CALL_TIMEOUT_MS stopped working — an operator override that
  // still parsed, still logged, and did nothing.
  const timeouts = timeoutsFromEnv("HOTLINE_PI");
  // Fences an in-flight hot apply against the session moving underneath it. A
  // pi apply awaits setModel, and the session can shut down inside that await;
  // a single `ctx !== null` check taken at entry then lets setThinkingLevel and
  // the harness_info restamp run against a session that is already gone.
  // Bumped on both session boundaries and on child ready (a respawned child is
  // a new result channel).
  let sessionGeneration = 0;
  let ctx: ExtensionContext | null = null;
  // FB13: disposer for the pi-subagents → `hotline job` event bridge (set in
  // session_start when auto job cards are enabled, torn down in session_shutdown).
  let disposeJobCards: (() => void) | null = null;
  // The uncapped hotline AgentInstructions, delivered in the initialize result
  // and re-appended to the system prompt every turn via before_agent_start.
  let hotlineInstructions = "";
  // Tool names already registered with Pi. We register each name once; the
  // execute closure reads the *live* client from the manager, so it keeps
  // working across child respawns without re-registration.
  const registeredTools = new Set<string>();

  // Mission Control handoff loop (spec §4/§5). Created in session_start; null
  // when no soft cap is set AND we only ride compaction (still created so layers
  // 2/3 run, but the cap layer is inert). A pending nudge line is consumed by
  // before_agent_start. `missionHandoffSeen` tracks whether the agent called the
  // mission handoff tool during the current handoff turn.
  let mcLoop: MissionControlLoop | null = null;
  let pendingNudge = "";
  let missionHandoffSeen = false;
  let lastUserMsg = "";
  const hotlineBinary = process.env.HOTLINE_BIN || "hotline";

  // ---- Box identity (§5.3 harness_info) + hot apply (pi hot-apply amendment
  // 2026-07-20) -------------------------------------------------------------
  //
  // The canonical "provider/id" of the model pi is running. Seeded from
  // ctx.model at session_start and kept current by the model_select event — we
  // do NOT re-read ctx.model, because that context object is the one handed to
  // session_start and is not guaranteed to track later selections.
  let currentModel: string | undefined;
  // The restamp-storm guard: setModel/setThinkingLevel fire model_select /
  // thinking_level_select themselves, so a hot apply would otherwise restamp
  // two or three times, and pi's TUI model cycle can fire repeatedly on the
  // same model. See IdentityAnnouncer.
  const announcer = new IdentityAnnouncer((info) => {
    const client = manager?.getClient();
    // Child down mid-respawn: report the drop so the announcement is not
    // recorded and the next child-ready resends it.
    if (!client) return false;
    client.notify(HARNESS_INFO_NOTIFICATION, info);
    log.info(`identity: harness_info model=${info.model ?? "(unknown)"} effort=${info.effort ?? "(unknown)"}`);
    return true;
  });

  function currentThinking(): PiThinkingLevel | undefined {
    try {
      return pi.getThinkingLevel() as PiThinkingLevel;
    } catch (err) {
      log.warn(`identity: getThinkingLevel failed: ${String(err)}`);
      return undefined;
    }
  }

  /**
   * Report harness kind + the RESOLVED model/thinking to the run child, so the
   * box can stamp welcome/agent_state and the app's chip and AGENT rows follow.
   *
   * Deduped by announced content: a restamp that says nothing new is dropped.
   * `force` bypasses the guard for a freshly-ready child, which starts with
   * only the box's config seed and needs the live values regardless.
   */
  function sendHarnessInfo(force = false): void {
    announcer.announce(currentModel, currentThinking(), force);
  }

  /**
   * Enumerate the models the app should offer and report them to the run child
   * (model catalog amendment 2026-07-20).
   *
   * Sent on child-ready ONLY — not on every identity restamp. harness_info
   * fires on every model change, every effort change, and every Ctrl+P cycle;
   * a model list stapled to it would re-send the whole catalog several times
   * per apply to say nothing new. The catalog changes when the harness restarts
   * or its registry moves, and a restart IS a child-ready, so once per
   * child-ready covers every case that matters. The box dedupes an identical
   * report anyway, so a respawn costs one frame and no device traffic.
   *
   * Fire-and-forget: the build never throws, an empty catalog is a legitimate
   * "nothing to say", and the box treats absence as "keep the curated list".
   */
  function sendHarnessCatalog(): void {
    const registry = ctx?.modelRegistry;
    const sessionCtx = ctx;
    if (!registry || !sessionCtx) return;
    void buildCatalog({
      patterns: parsePiModels(process.env.HOTLINE_PI_MODELS),
      enabledModels: () => readPiEnabledModels(sessionCtx),
      resolveScope: (patterns) => resolveModelScopeWithDiagnostics(patterns, registry),
      getAvailable: () => registry.getAvailable() as PiModelLike[],
      hasConfiguredAuth: (model) => registry.hasConfiguredAuth(model as never),
      log,
    })
      .then((catalog) => {
        const client = manager?.getClient();
        // Child down mid-respawn: drop it. The next child-ready rebuilds and
        // resends — there is no state to reconcile, the catalog is stateless.
        if (!client) {
          log.warn("catalog: child down; report dropped");
          return;
        }
        client.notify(HARNESS_CATALOG_NOTIFICATION, catalog);
        log.info(
          `catalog: ${catalog.models.length} model(s) from ${catalog.source}` +
            `${catalog.truncated ? " (truncated)" : ""}`,
        );
      })
      .catch((err: unknown) => log.warn(`catalog: report failed: ${String(err)}`));
  }

  /** Canonical "provider/id" for a pi Model object. */
  function canonicalModelId(model: { provider?: unknown; id?: unknown } | undefined): string | undefined {
    if (!model) return undefined;
    const provider = typeof model.provider === "string" ? model.provider : "";
    const id = typeof model.id === "string" ? model.id : "";
    if (id === "") return undefined;
    return provider === "" ? id : `${provider}/${id}`;
  }

  /**
   * Resolve a model pattern through pi's OWN resolver — the same
   * `resolveCliModel` the `--provider/--model` flags go through, against the
   * live ModelRegistry. That is the point: a string that works on the box's
   * launch line works from the app, including bare ids ("gpt-5.6-sol"),
   * provider-scoped ids ("openai-codex/gpt-5.6-sol"), fuzzy patterns, and a
   * trailing ":level". Returns null when nothing matches.
   */
  function resolveModelPattern(pattern: string): PiResolvedModel | null {
    const registry = ctx?.modelRegistry;
    if (!registry) return null;
    const res = resolveCliModel({ cliModel: pattern, modelRegistry: registry });
    if (res.error !== undefined || res.model === undefined) {
      if (res.error !== undefined) log.warn(`sdk_apply: ${res.error}`);
      return null;
    }
    const canonical = canonicalModelId(res.model);
    if (canonical === undefined) return null;
    if (res.warning !== undefined) log.warn(`sdk_apply: ${res.warning}`);
    return {
      model: res.model,
      provider: res.model.provider,
      id: res.model.id,
      canonical,
      thinkingLevel: res.thinkingLevel as PiThinkingLevel | undefined,
    };
  }

  const handleSdkApply = createPiApplyHandler({
    resolveModel: resolveModelPattern,
    setModel: (model) => pi.setModel(model as never),
    getThinkingLevel: () => currentThinking() ?? "off",
    setThinkingLevel: (level) => pi.setThinkingLevel(level),
    getSession: () => (ctx === null ? null : sessionGeneration),
    notifyResult: (result) => {
      const client = manager?.getClient();
      if (!client) {
        // Child down mid-respawn: the box's pending timer surfaces
        // harness_unreachable; nothing was persisted, retry is safe.
        log.warn(`sdk_apply ${result.rid}: child down; result dropped`);
        return;
      }
      client.notify(SDK_APPLY_RESULT_NOTIFICATION, result);
    },
    onApplied: (applied) => {
      // Mirror into this process's env so every `hotline` subprocess we spawn
      // from here (pi.exec for mission/job) reports the same knobs the box
      // just confirmed, rather than the values `up` exported at launch.
      //
      // Scope, stated honestly: this does NOT reach a respawned `hotline run`
      // child — ChildManager captured its env object at session_start and
      // reuses it per spawn. That is fine here, unlike on the claude-sdk path
      // where buildOptions re-reads the env: the run child uses HOTLINE_PI_*
      // only to SEED its identity, and our harness_info overwrites that seed
      // within a beat of the child becoming ready. The durable record is the
      // box's .env write (persist-after-ok), not this mirror.
      //
      // A cleared ("") field UNPINS: pi keeps running its live value, we just
      // stop asserting one.
      if (applied.model !== undefined) {
        if (applied.model === "") delete process.env.HOTLINE_PI_MODEL;
        else process.env.HOTLINE_PI_MODEL = applied.model;
        // The persisted id carries its own provider; a stale HOTLINE_PI_PROVIDER
        // would be emitted as an explicit --provider that fights the prefix
        // (config.UpdatePiEnv removes the .env line for the same reason).
        delete process.env.HOTLINE_PI_PROVIDER;
      }
      if (applied.effort !== undefined) {
        if (applied.effort === "") delete process.env.HOTLINE_PI_THINKING;
        else process.env.HOTLINE_PI_THINKING = applied.effort;
      }
      // The model_select/thinking_level_select events pi fired for our own
      // setModel/setThinkingLevel have already updated currentModel; this
      // restamp is usually deduped to a no-op by them, and is kept as the
      // ordering guarantee that identity precedes the ok result.
      if (applied.model !== undefined && applied.model !== "") currentModel = applied.model;
      sendHarnessInfo();
    },
    log,
  });

  function setStatus(text: string | undefined): void {
    if (ctx?.hasUI) {
      try {
        ctx.ui.setStatus("hotline", text);
      } catch {
        // UI may be a no-op in some modes; never let it break the bridge.
      }
    }
  }

  function notifyUser(message: string, level: "info" | "warning" | "error"): void {
    if (ctx?.hasUI) {
      try {
        ctx.ui.notify(message, level);
      } catch {
        // ignore
      }
    }
  }

  function registerTools(tools: McpTool[]): void {
    for (const tool of tools) {
      if (registeredTools.has(tool.name)) continue;
      registeredTools.add(tool.name);
      const description = tool.description ?? tool.name;
      pi.registerTool({
        name: tool.name,
        label: tool.name,
        description,
        promptSnippet: firstSentence(description),
        // Raw JSON Schema passes straight through to the provider (probe 1);
        // Pi does no client-side arg validation, so the child's schema literal
        // is used verbatim. The `as never` satisfies the TSchema param type.
        parameters: (tool.inputSchema ?? { type: "object", properties: {} }) as never,
        async execute(_toolCallId: string, params: unknown) {
          const client = manager?.getClient();
          if (!client) {
            // Surfaced to the model as a tool error (throwing sets isError).
            throw new Error(
              "hotline channel is down (reconnecting). The message was not delivered; retry shortly.",
            );
          }
          const res = await mcpCallTool(client, tool.name, params ?? {}, timeouts.callTimeoutMs);
          const text = (res.content ?? [])
            .map((c) => (c.type === "text" ? c.text ?? "" : ""))
            .join("");
          if (res.isError) {
            // hotline reports tool-level failures via isError; map to a Pi tool
            // error by throwing (a returned value never sets isError).
            throw new Error(text || `${tool.name} failed`);
          }
          return { content: [{ type: "text", text }], details: {} };
        },
      } as never);
    }
  }

  function onReady(_client: unknown, init: InitializeResult, tools: McpTool[]): void {
    hotlineInstructions = (init.instructions ?? "").trim();
    // A respawned child is a new result channel: an apply captured against the
    // old one can no longer be answered on it.
    sessionGeneration += 1;
    registerTools(tools);
    setStatus("hotline: connected");
    // A respawned child starts with only the box's config seed; force the
    // identity through the dedupe guard so it always gets the live values.
    sendHarnessInfo(true);
    // …and the selectable set behind it, so the app's model row shows THIS
    // box's models rather than a list compiled into the client.
    sendHarnessCatalog();
    log.info(
      `ready: instructions ${hotlineInstructions.length} chars, ${registeredTools.size} tool(s) registered`,
    );
  }

  function onNotification(method: string, params: unknown): void {
    if (method === SDK_APPLY_NOTIFICATION) {
      // Hot model/effort apply: async, never blocks the notification
      // dispatcher; every path settles by notifying an sdk_apply_result.
      void handleSdkApply(params);
      return;
    }
    if (method !== CHANNEL_NOTIFICATION) {
      log.warn(`ignoring unexpected notification: ${method}`);
      return;
    }
    // The pi sink pre-renders the full <channel …> envelope into params.content
    // and passes meta=nil (run_pi.go). We forward the content verbatim as a user
    // turn; everything the model needs to route (chat_id, source) is in the text.
    const content = (params as { content?: string } | null)?.content;
    if (typeof content !== "string" || content === "") {
      log.warn("channel notification had empty content; dropping");
      return;
    }

    deliverToAgent(content);
  }

  // Forward one channel envelope to the agent as a user turn. The retry ladder
  // (idle bare send → steer on the mid-stream race → drop loudly) lives in
  // deliver.ts so it can be unit-tested; see that file for why every path is
  // caught (this runs inside the child's stdout data handler) and why the
  // already-processing signal is handled on the async rejection path on 0.80.5.
  function deliverToAgent(content: string): void {
    lastUserMsg = content;
    const streaming = ctx ? !ctx.isIdle() : false;
    deliverInbound(pi, content, streaming, log);
  }

  function onFatal(message: string): void {
    setStatus("hotline: error");
    notifyUser(message, "error");
  }

  function onDown(reason: string): void {
    setStatus("hotline: reconnecting");
    log.warn(`hotline down: ${reason}`);
  }

  // Bidirectional sync (pi hot-apply amendment 2026-07-20) — the win pi's
  // topology gives us that the claude-sdk harness cannot have. pi is the HOST
  // here, so when George changes model with Ctrl+P or thinking level in the
  // TUI, we hear about it and restamp harness_info; the app's chip and AGENT
  // rows follow a change made in the terminal.
  //
  // Both handlers are also fired by our OWN setModel/setThinkingLevel during a
  // hot apply. That is fine and is exactly what the sendHarnessInfo dedupe
  // guard is for: the second and third restamps of one apply announce nothing
  // new and are dropped, so a combined model+effort apply emits ONE
  // harness_info, not three.
  pi.on("model_select", async (event) => {
    const canonical = canonicalModelId(event.model);
    if (canonical === undefined) return;
    currentModel = canonical;
    sendHarnessInfo();
  });

  pi.on("thinking_level_select", async () => {
    sendHarnessInfo();
  });

  pi.on("session_start", async (_event, sessionCtx) => {
    ctx = sessionCtx;
    sessionGeneration += 1;
    // Seed the identity from the session's starting model. From here the
    // model_select event is the source of truth.
    currentModel = canonicalModelId(sessionCtx.model);
    announcer.reset();
    // Defensive: if a prior manager is still live (a second session_start
    // without an intervening session_shutdown, or a shutdown-ordering surprise),
    // tear it down first so we never leak a second `hotline run` child competing
    // for the box/runtime ownership locks.
    if (manager) {
      log.warn("session_start with a live manager; stopping it before rebinding");
      manager.stop();
      manager = null;
      registeredTools.clear();
    }
    // The Go child selects its pi branch from HOTLINE_HARNESS; force it here so
    // the community-mode spawn (no supervisor) also takes run_pi.go. Under
    // `hotline up` the supervisor already exports HOTLINE_HARNESS=pi and
    // HOTLINE_SUPERVISOR_DIR, both of which flow through untouched.
    const env: NodeJS.ProcessEnv = { ...process.env, HOTLINE_HARNESS: "pi" };
    const binary = process.env.HOTLINE_BIN || "hotline";

    manager = new ChildManager({
      binary,
      args: ["run"],
      env,
      clientName: "hotline-pi",
      timeouts,
      installHint: "See the hotline-pi README.",
      log,
      onNotification,
      onReady,
      onDown,
      onFatal,
    });
    setStatus("hotline: starting");
    manager.start();

    // Mission Control handoff loop: wire the pure decision core to live pi
    // actions. Every action is defensively caught — this runs inside pi's event
    // loop and must never throw a session down.
    pendingNudge = "";
    missionHandoffSeen = false;
    const configuredCap = parseContextCap();
    const keepRecentTokens = configuredCap == null ? null : readPiKeepRecentTokens(sessionCtx);
    const cap = clampContextCap(configuredCap, keepRecentTokens);
    if (
      configuredCap != null &&
      keepRecentTokens != null &&
      configuredCap <= keepRecentTokens + COMPACTION_CAP_MARGIN
    ) {
      log.warn(
        `mission: HOTLINE_MC_CONTEXT_CAP=${configuredCap} leaves too little compactable history above pi keepRecentTokens=${keepRecentTokens}; using minimum effective cap ${cap} (${COMPACTION_CAP_MARGIN}-token safety margin)`,
      );
    }
    const actions: LoopActions = {
      armNudge(line) {
        pendingNudge = line;
      },
      compact(customInstructions) {
        const compactCtx = ctx;
        if (!compactCtx) return Promise.reject(new Error("session context is unavailable"));
        return compactWithCallbacks(compactCtx, customInstructions);
      },
      sendHandoffTurn(prompt) {
        try {
          Promise.resolve(pi.sendUserMessage(prompt, { deliverAs: "followUp" })).catch(
            (err: unknown) => log.warn(`mission: handoff turn send rejected: ${String(err)}`),
          );
        } catch (err) {
          log.warn(`mission: handoff turn send threw: ${String(err)}`);
        }
      },
      mechanicalHandoff(state, next) {
        pi.exec(hotlineBinary, ["mission", "handoff", "--trigger", "auto", "--state", state, "--next", next])
          .catch((err: unknown) => log.warn(`mission: mechanical handoff failed: ${String(err)}`));
      },
      log(msg) {
        log.info(`mission: ${msg}`);
      },
      warn(msg) {
        log.warn(`mission: ${msg}`);
      },
    };
    mcLoop = new MissionControlLoop(cap, actions);
    log.info(`mission: handoff loop armed (cap ${cap ?? "unset"})`);

    // FB13: bridge pi-subagents lifecycle events to `hotline job` so every
    // subagent the bot fans out shows up as a live job card in the app channel.
    // Only wired here, on the top-level bot's session (a depth>0 worker returned
    // early above and never reaches this handler), so worker subagents — which
    // run in their own pi process on their own bus — are never double-carded.
    // Best-effort throughout; a card failure never disturbs the run.
    if (disposeJobCards) {
      disposeJobCards();
      disposeJobCards = null;
    }
    if (autoJobsEnabled()) {
      // batch = pi session id: the stable per-run key. pi-subagents does not put
      // its group id on the lifecycle payloads, so the session id is the fallback.
      let batch = "";
      try {
        batch = sessionCtx.sessionManager.getSessionId() || "";
      } catch {
        batch = "";
      }
      if (batch) {
        disposeJobCards = wireJobCards(pi, { binary, batch });
      } else {
        log.warn("job cards: no session id available; not wiring subagent cards");
      }
    }
  });

  pi.on("before_agent_start", async (event) => {
    if (typeof event.prompt === "string" && event.prompt) lastUserMsg = event.prompt;
    // Chained across extensions: append hotline's mechanics+voice contract, plus
    // any armed Mission Control nudge (consumed once), to whatever system prompt
    // earlier handlers produced. Re-supplied every turn by design.
    let prompt = event.systemPrompt;
    if (hotlineInstructions) prompt += "\n\n" + hotlineInstructions;
    if (pendingNudge) {
      prompt += "\n\n" + pendingNudge;
      pendingNudge = "";
    }
    if (prompt === event.systemPrompt) return;
    return { systemPrompt: prompt };
  });

  // Mission Control: read the context estimate each turn to arm the 80% nudge and
  // enforce the soft cap; observe the mission handoff tool call to skip the
  // mechanical fallback; drive the cancel→handoff-turn→recompact loop.
  pi.on("agent_settled", async () => {
    if (!mcLoop) return;
    // Both callbacks receive one explicit generation. onContextUsage may queue a
    // follow-up handoff turn, but onHandoffTurnSettled refuses to consume work
    // armed in this same generation; only the follow-up's later settle can do so.
    const settleSeq = mcLoop.beginAgentSettle();
    try {
      const usage = ctx?.getContextUsage();
      if (usage) mcLoop.onContextUsage(usage, settleSeq);
    } catch (err) {
      log.warn(`mission: context usage read failed: ${String(err)}`);
    }
    try {
      mcLoop.onHandoffTurnSettled(missionHandoffSeen, lastUserMsg, settleSeq);
    } catch (err) {
      log.warn(`mission: handoff-turn settle failed: ${String(err)}`);
    }
    missionHandoffSeen = false;
  });

  pi.on("tool_result", async (event) => {
    if (!mcLoop) return;
    if (event.toolName === "mission" && !event.isError) {
      const action = (event.input as { action?: unknown } | null)?.action;
      if (action === "handoff") {
        missionHandoffSeen = true;
        mcLoop.noteHandoffWritten();
      }
    }
  });

  pi.on("session_before_compact", async (event) => {
    if (!mcLoop) return;
    try {
      const { cancel } = mcLoop.onBeforeCompact(event.reason);
      // Pi 0.80.6 copies customInstructions into this event, but after handlers it
      // passes the original local variable to the built-in summarizer; mutation
      // here would only mislead later extensions. Our recompact path supplies the
      // instruction through ctx.compact(), the supported direct seam. Pi-owned
      // compactions rely on the handoff already written to Mission Control disk.
      if (cancel) return { cancel: true };
    } catch (err) {
      log.warn(`mission: before_compact failed: ${String(err)}`);
    }
    return;
  });

  pi.on("session_compact", async () => {
    mcLoop?.onCompacted();
  });

  pi.on("session_shutdown", async () => {
    log.info("session_shutdown: stopping hotline child");
    if (disposeJobCards) {
      disposeJobCards();
      disposeJobCards = null;
    }
    manager?.stop();
    manager = null;
    registeredTools.clear();
    hotlineInstructions = "";
    mcLoop = null;
    pendingNudge = "";
    missionHandoffSeen = false;
    ctx = null;
    sessionGeneration += 1;
    currentModel = undefined;
    announcer.reset();
  });
}
