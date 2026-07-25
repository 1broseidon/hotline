/**
 * The Agent SDK session loop (spec §5): assemble Options, own the query()
 * lifecycle, consume its message stream, persist the session id.
 *
 * The streaming-input generator stays open for the process lifetime; inbound
 * channel envelopes are yielded as user turns and the SDK queues any that
 * arrive mid-turn. If the stream ends while we are not shutting down, the
 * caller exits non-zero and the Go supervisor respawns us — `resume` plus the
 * child's replayCatchup restore continuity.
 */

import { query, type Options, type Query, type SDKMessage, type SDKUserMessage } from "@anthropic-ai/claude-agent-sdk";
import type { AsyncQueue } from "@1broseidon/hotline-harness-core/queue";
import { saveSessionId, clearSessionId } from "@1broseidon/hotline-harness-core/session";
import type { SdkMcpServerConfig } from "./proxy.js";
import { effortToSdkApply, parsePositiveInt, describeEffort, parseSettingSources, type SdkEffortLevel } from "./options.js";
import { log } from "./log.js";

export interface RunAgentDeps {
  queue: AsyncQueue<SDKUserMessage>;
  proxy: SdkMcpServerConfig;
  /** The uncapped hotline AgentInstructions from the child's initialize. */
  instructions: string;
  sessionFile: string;
  savedSessionId: string | null;
  abortController: AbortController;
  /** True once shutdown has been requested (SIGTERM/SIGINT). */
  isShuttingDown: () => boolean;
  /**
   * Called once per query with the SDK-RESOLVED model id (the init message's
   * `model`) — the wire-metadata refinement hook (harness_info, spec §5.3).
   */
  onInit?: (resolvedModel: string) => void;
  /**
   * Live Query handle tracking (SDK hot-model amendment 2026-07-19): called
   * with the handle right after query() starts and with null when the stream
   * ends or throws, so the host can route hot-apply controls (setModel) at
   * the running session and refuse (no_session) between sessions.
   */
  onQuery?: (q: Query | null) => void;
  /**
   * The query factory. Defaults to the Agent SDK's `query`; overridable so the
   * retry loop's queue handling — the part that used to silently drop inbound
   * turns — can be driven against a scripted failing session in tests. Nothing
   * in production passes this.
   */
  queryFn?: typeof query;
  /**
   * Observer called once per stream message, before this loop's own
   * branching on `msg.type`/`subtype` (job cards, spec workstream 1: the SDK's
   * task_started/task_updated system messages are a better event source than
   * pi's extension bus, and this hook is how jobcards.ts sees them). Guarded
   * in try/catch here so a throwing observer never takes the session down.
   */
  onMessage?: (msg: SDKMessage) => void;
  /**
   * Mission control hooks (spec workstream 2): UserPromptSubmit (80% nudge),
   * PreCompact (CLI-auto mechanical handoff), PostToolUse[mcp__hotline__mission]
   * (handoff observation). Merged verbatim into Options.hooks. Undefined in
   * tests and when the loop is off.
   */
  hooks?: Options["hooks"];
  /**
   * Mission control settle (spec workstream 2): called once per `result`
   * message with the live Query handle so the host can poll getContextUsage()
   * and step the loop. Awaited in the stream loop (the session is idle between
   * turns, the same seam pi's agent_settled uses) and guarded so it never
   * takes the session down.
   */
  onResult?: (query: Query) => void | Promise<void>;
  /**
   * Mission control reset (spec workstream 2): called on the compact_boundary
   * system message so the loop returns to an idle cycle. Guarded here.
   */
  onCompactBoundary?: (trigger: "manual" | "auto") => void;
}

function buildOptions(deps: RunAgentDeps, resume: string | undefined): Options {
  // Per-box knobs (spec §3): `hotline up` resolved real env + shared .env
  // (and the deprecated HOTLINE_CLAUDE_SDK_MODEL fallback) into these
  // canonical vars; we read real env only.
  const model = process.env.HOTLINE_SDK_MODEL || undefined;
  // Effort binds through the SAME mapping the hot path uses (effortToSdkApply):
  // a symbolic name → Options.effort (the modern control), a positive integer →
  // Options.maxThinkingTokens. Sharing one mapping means a hot-applied effort
  // survives a later respawn on the identical control — no boot/hot drift.
  const effortApply = effortToSdkApply(process.env.HOTLINE_SDK_EFFORT);
  const effort: SdkEffortLevel | undefined =
    effortApply.kind === "effortLevel" ? effortApply.level : undefined;
  const maxThinkingTokens: number | undefined =
    effortApply.kind === "maxThinkingTokens" ? effortApply.tokens : undefined;
  const maxTurns = parsePositiveInt(process.env.HOTLINE_SDK_MAX_TURNS);
  // settingSources default is [] (hermetic). HOTLINE_SDK_SETTING_SOURCES opens
  // specific filesystem tiers (e.g. "project" to load CLAUDE.md when migrating
  // an operator's own agent onto the harness); an invalid token throws here and
  // crashes the boot loudly (the `up` path already pre-validated it).
  const settingSources = parseSettingSources(process.env.HOTLINE_SDK_SETTING_SOURCES);
  log.info(
    `sdk options: model=${model ?? "default"} effort=${describeEffort(process.env.HOTLINE_SDK_EFFORT)} maxTurns=${maxTurns ?? "unlimited"} settingSources=[${settingSources.join(",")}]`,
  );
  return {
    cwd: process.cwd(), // cmd_up runs us in the box workdir
    mcpServers: { hotline: deps.proxy },
    allowedTools: ["mcp__hotline__*"],
    permissionMode: "bypassPermissions", // pi posture: headless, no prompt possible
    allowDangerouslySkipPermissions: true,
    // The harness OWNS the "hotline" MCP server (the in-process proxy above) and
    // its `hotline run` child. strictMcpConfig makes Options.mcpServers the ONLY
    // MCP source, so a project .mcp.json registering its own "hotline" server
    // (as the orchestrator box's project config does) is ignored even when
    // settingSources includes "project" — no name collision, no second
    // `hotline run` tripping the box ownership guard. Maps to the CLI
    // --strict-mcp-config flag (sdk.d.ts). See TASK 2.
    strictMcpConfig: true,
    settingSources, // hermetic [] by default; HOTLINE_SDK_SETTING_SOURCES opts tiers in
    systemPrompt: { type: "preset", preset: "claude_code", append: deps.instructions },
    resume,
    persistSession: true,
    model,
    effort,
    maxThinkingTokens,
    maxTurns,
    hooks: deps.hooks, // mission control (spec workstream 2); undefined = none
    env: process.env as Record<string, string>, // auth flows through (AnthropicChildEnv applied by cmd_up)
    abortController: deps.abortController,
    stderr: (line: string) => log.info(`cc: ${line.trimEnd()}`),
  };
}

function summarize(msg: SDKMessage): void {
  if (msg.type === "assistant") {
    const blocks =
      (msg as unknown as { message?: { content?: Array<Record<string, unknown>> } }).message?.content ?? [];
    const parts: string[] = [];
    for (const b of blocks) {
      if (b.type === "text" && typeof b.text === "string") parts.push(b.text.slice(0, 120));
      else if (b.type === "tool_use" && typeof b.name === "string") parts.push(`[tool_use ${b.name}]`);
    }
    if (parts.length > 0) log.info(`assistant: ${parts.join(" | ")}`);
  }
}

/**
 * Run the session to completion. Returns the exit code the process should use:
 * 0 for a clean shutdown, 1 for an unexpected stream end (supervisor respawns).
 */
export async function runAgent(deps: RunAgentDeps): Promise<number> {
  let resume = deps.savedSessionId ?? undefined;
  let attempt = 0;

  for (;;) {
    attempt++;
    let sawMessage = false;
    let loggedInit = false;
    // One LEASED consumer per attempt (sol review #5). The SDK pulls turns from
    // the generator before it produces anything, so a resume that fails throws
    // with those turns already gone — and the retry below builds a second
    // consumer over the same queue. Leasing means an abandoned attempt gives
    // its turns back instead of dropping them, and cancel() releases the dead
    // iterator's waiter so it cannot strand.
    const consumer = deps.queue.consumer();
    try {
      const generator = async function* (): AsyncIterable<SDKUserMessage> {
        for await (const m of consumer) yield m;
      };
      const startQuery = deps.queryFn ?? query;
      const q = startQuery({ prompt: generator(), options: buildOptions(deps, resume) });
      deps.onQuery?.(q);

      for await (const msg of q) {
        // Every message is proof the session is consuming what it pulled, so
        // each one releases the outstanding leases. Before the FIRST message
        // nothing is proven — that is exactly the window a failed resume
        // throws in, and every turn pulled inside it is still owed back.
        consumer.ack();
        sawMessage = true;
        if (deps.onMessage) {
          try {
            deps.onMessage(msg);
          } catch (err) {
            log.warn(`onMessage hook threw: ${(err as Error).message}`);
          }
        }
        const sessionId = (msg as { session_id?: string }).session_id;
        if (msg.type === "system") {
          if (sessionId) saveSessionId(deps.sessionFile, sessionId);
          const sys = msg as {
            subtype?: string;
            model?: string;
            compact_metadata?: { trigger?: "manual" | "auto" };
          };
          // The truth line (spec §3.4): the SDK's init message carries the
          // RESOLVED model id — what the session actually runs, whatever the
          // knob said. Logged once per query.
          const initModel = sys.subtype === "init" ? sys.model : undefined;
          if (!loggedInit && initModel) {
            loggedInit = true;
            log.info(`sdk session: model=${initModel} session=${sessionId ?? "?"}`);
            try {
              deps.onInit?.(initModel);
            } catch (err) {
              log.warn(`onInit hook threw: ${(err as Error).message}`);
            }
          } else if (sys.subtype === "compact_boundary" && deps.onCompactBoundary) {
            // Mission control reset (spec workstream 2): compaction has landed.
            try {
              deps.onCompactBoundary(sys.compact_metadata?.trigger ?? "auto");
            } catch (err) {
              log.warn(`onCompactBoundary hook threw: ${(err as Error).message}`);
            }
          }
        } else if (msg.type === "result") {
          const r = msg as { subtype?: string; duration_ms?: number; total_cost_usd?: number };
          log.info(
            `result: ${r.subtype ?? "?"} duration_ms=${r.duration_ms ?? "?"} cost_usd=${r.total_cost_usd ?? "?"}`,
          );
          if (sessionId) saveSessionId(deps.sessionFile, sessionId);
          // Mission control settle (spec workstream 2): poll context usage and
          // step the loop. Awaited — the session is idle between turns — but
          // guarded so a getContextUsage failure never ends the session.
          if (deps.onResult) {
            try {
              await deps.onResult(q);
            } catch (err) {
              log.warn(`onResult hook threw: ${(err as Error).message}`);
            }
          }
        } else {
          summarize(msg);
        }
      }

      // Stream ended: generator closed (shutdown) or the runtime went away.
      if (deps.isShuttingDown()) {
        log.info("agent stream ended during shutdown");
        return 0;
      }
      log.error("agent stream ended unexpectedly; exiting for supervisor respawn");
      return 1;
    } catch (err) {
      if (deps.isShuttingDown()) {
        log.info(`agent stream aborted during shutdown: ${(err as Error).message}`);
        return 0;
      }
      // A resume against a wiped/missing session errors before any message.
      // One retry without resume (spec §5), then give up to the supervisor.
      //
      // The turns the doomed query's generator already pulled (replayCatchup
      // can push an unbounded number at boot) are handed BACK to the queue
      // here, and the dead generator's waiter is released, so the retry's
      // consumer sees them first and in order. Losing them used to be silent:
      // the operator's messages simply never reached the agent.
      if (resume !== undefined && !sawMessage && attempt === 1) {
        log.warn(`resume of session ${resume} failed (${(err as Error).message}); retrying without resume`);
        consumer.cancel();
        clearSessionId(deps.sessionFile);
        resume = undefined;
        continue;
      }
      log.error(`agent session failed: ${(err as Error).message}`);
      return 1;
    } finally {
      // The handle is dead on every exit from this attempt (return, the
      // retry-without-resume continue, throw): clear it so a hot apply
      // between sessions is refused as no_session instead of poking a dead
      // stream.
      deps.onQuery?.(null);
      // Deliberately NOT cancelling here. Requeuing on a give-up exit would be
      // pointless (the process exits and the queue dies with it) and actively
      // wrong for the clean-shutdown path, where a turn the session pulled and
      // is finishing would be handed back and later re-delivered. The retry
      // path — the one case where the queue outlives the attempt — cancels
      // explicitly above.
    }
  }
}
