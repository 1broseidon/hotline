/**
 * The Agent SDK session loop (design §3): assemble Options, own the query()
 * lifecycle, consume its message stream, persist the session id, and drive the
 * turn ledger's stream-side transitions (T2/T4/T6/T8) plus M2 auth
 * classification.
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
  onInit?: (resolvedModel: string) => void;
  onQuery?: (q: Query | null) => void;
  queryFn?: typeof query;
  onMessage?: (msg: SDKMessage) => void;
  hooks?: Options["hooks"];
  onResult?: (query: Query) => void | Promise<void>;
  onCompactBoundary?: (trigger: "manual" | "auto") => void;

  // ---- M1 turn ledger (design §3.4) --------------------------------------
  /** T2/T3: a user message echoed on the stream (uuid attribution, echo mode). */
  onUserEcho?: (uuid: string | undefined, isReplay: boolean) => void;
  /** Pull-window attribution (design §3.5): the SDK pulled a message from the
   * generator. Wired instead of onUserEcho when echo mode is off. */
  onPull?: (uuid: string | undefined) => void;
  /** T4: assistant text blocks buffered for the turn. */
  onAssistantText?: (texts: string[]) => void;
  /** T6 settle: fire the ledger's settle plan (fallback lane). Awaited — the
   * session is idle between turns — and guarded so it never ends the session. */
  onSettle?: (subtype: string) => void | Promise<void>;
  /** T8 teardown: the retry-without-resume path requeues leases; the ledger
   * reverts DELIVERED-not-SETTLED envelopes so the retry re-delivers them. */
  onTeardown?: () => void;

  // ---- M2 auth containment (design §4) -----------------------------------
  /** Classify a stream message as an auth failure (or null). */
  classifyAuthFailure?: (msg: SDKMessage) => string | null;
  /** Classify a thrown stream error as auth-shaped (or null), used only before
   * any result. */
  classifyAuthThrow?: (err: unknown) => string | null;
  /** A successful init cleared the credential problem — reset the counter. */
  onAuthReset?: () => void;
  /** Escalate a classified auth failure; returns the process exit code. */
  onAuthFailure?: (errorText: string) => Promise<number>;
}

function buildOptions(deps: RunAgentDeps, resume: string | undefined): Options {
  const model = process.env.HOTLINE_SDK_MODEL || undefined;
  const effortApply = effortToSdkApply(process.env.HOTLINE_SDK_EFFORT);
  const effort: SdkEffortLevel | undefined =
    effortApply.kind === "effortLevel" ? effortApply.level : undefined;
  const maxThinkingTokens: number | undefined =
    effortApply.kind === "maxThinkingTokens" ? effortApply.tokens : undefined;
  const maxTurns = parsePositiveInt(process.env.HOTLINE_SDK_MAX_TURNS);
  const settingSources = parseSettingSources(process.env.HOTLINE_SDK_SETTING_SOURCES);
  log.info(
    `sdk options: model=${model ?? "default"} effort=${describeEffort(process.env.HOTLINE_SDK_EFFORT)} maxTurns=${maxTurns ?? "unlimited"} settingSources=[${settingSources.join(",")}]`,
  );
  return {
    cwd: process.cwd(), // cmd_up runs us in the box workdir
    mcpServers: { hotline: deps.proxy },
    // NOTE (design §3.6): the old harness set allowedTools: ["mcp__hotline__*"],
    // a misleading no-op — with bypassPermissions it auto-allows everything, so
    // the field whitelisted nothing. Dropped here; M3 wires a real permission
    // handler on the relay branch.
    permissionMode: "bypassPermissions", // headless, no prompt possible (M3 changes this)
    allowDangerouslySkipPermissions: true,
    strictMcpConfig: true,
    settingSources, // hermetic [] by default; HOTLINE_SDK_SETTING_SOURCES opts tiers in
    systemPrompt: { type: "preset", preset: "claude_code", append: deps.instructions },
    resume,
    persistSession: true,
    model,
    effort,
    maxThinkingTokens,
    maxTurns,
    hooks: deps.hooks,
    env: process.env as Record<string, string>,
    abortController: deps.abortController,
    stderr: (line: string) => log.info(`cc: ${line.trimEnd()}`),
  };
}

/** Extract the text blocks of an assistant message for the ledger's textBuf. */
function assistantTextBlocks(msg: SDKMessage): string[] {
  const blocks =
    (msg as unknown as { message?: { content?: Array<Record<string, unknown>> } }).message?.content ?? [];
  const out: string[] = [];
  for (const b of blocks) {
    if (b.type === "text" && typeof b.text === "string") out.push(b.text);
  }
  return out;
}

function summarize(msg: SDKMessage): void {
  if (msg.type === "assistant") {
    const parts: string[] = [];
    for (const t of assistantTextBlocks(msg)) parts.push(t.slice(0, 120));
    const blocks =
      (msg as unknown as { message?: { content?: Array<Record<string, unknown>> } }).message?.content ?? [];
    for (const b of blocks) if (b.type === "tool_use" && typeof b.name === "string") parts.push(`[tool_use ${b.name}]`);
    if (parts.length > 0) log.info(`assistant: ${parts.join(" | ")}`);
  }
}

/**
 * Run the session to completion. Returns the exit code the process should use:
 * 0 clean shutdown, 1 unexpected stream end, or an auth-fatal code from
 * onAuthFailure (design §4).
 */
export async function runAgent(deps: RunAgentDeps): Promise<number> {
  let resume = deps.savedSessionId ?? undefined;
  let attempt = 0;
  // Auth classification persists across the resume-retry (both attempts hit the
  // same credential problem); reset on a successful init.
  let authClassified: string | null = null;

  for (;;) {
    attempt++;
    let sawMessage = false;
    let sawResult = false;
    let loggedInit = false;
    const consumer = deps.queue.consumer();
    try {
      const generator = async function* (): AsyncIterable<SDKUserMessage> {
        for await (const m of consumer) {
          deps.onPull?.((m as { uuid?: string }).uuid);
          yield m;
        }
      };
      const startQuery = deps.queryFn ?? query;
      const q = startQuery({ prompt: generator(), options: buildOptions(deps, resume) });
      deps.onQuery?.(q);

      for await (const msg of q) {
        consumer.ack();
        sawMessage = true;

        // M2: classify auth-shaped stream messages (design §4.1).
        if (!authClassified && deps.classifyAuthFailure) {
          const c = deps.classifyAuthFailure(msg);
          if (c) authClassified = c;
        }

        // M1 ledger stream-side transitions (T2/T4).
        if (msg.type === "user") {
          const u = msg as { uuid?: string; isReplay?: unknown };
          deps.onUserEcho?.(u.uuid, u.isReplay === true);
        } else if (msg.type === "assistant") {
          const texts = assistantTextBlocks(msg);
          if (texts.length > 0) deps.onAssistantText?.(texts);
        }

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
          const initModel = sys.subtype === "init" ? sys.model : undefined;
          if (!loggedInit && initModel) {
            loggedInit = true;
            // A successful init means the credentials are live — reset M2 state.
            authClassified = null;
            try {
              deps.onAuthReset?.();
            } catch (err) {
              log.warn(`onAuthReset hook threw: ${(err as Error).message}`);
            }
            log.info(`sdk session: model=${initModel} session=${sessionId ?? "?"}`);
            try {
              deps.onInit?.(initModel);
            } catch (err) {
              log.warn(`onInit hook threw: ${(err as Error).message}`);
            }
          } else if (sys.subtype === "compact_boundary" && deps.onCompactBoundary) {
            try {
              deps.onCompactBoundary(sys.compact_metadata?.trigger ?? "auto");
            } catch (err) {
              log.warn(`onCompactBoundary hook threw: ${(err as Error).message}`);
            }
          }
        } else if (msg.type === "result") {
          sawResult = true;
          const r = msg as { subtype?: string; duration_ms?: number; total_cost_usd?: number };
          log.info(
            `result: ${r.subtype ?? "?"} duration_ms=${r.duration_ms ?? "?"} cost_usd=${r.total_cost_usd ?? "?"}`,
          );
          if (sessionId) saveSessionId(deps.sessionFile, sessionId);
          // M1 T6 settle: fire the ledger's fallback plan before mission's
          // context poll (both awaited, both guarded).
          if (deps.onSettle) {
            try {
              await deps.onSettle(r.subtype ?? "");
            } catch (err) {
              log.warn(`onSettle hook threw: ${(err as Error).message}`);
            }
          }
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
      if (authClassified && deps.onAuthFailure) {
        return await deps.onAuthFailure(authClassified);
      }
      log.error("agent stream ended unexpectedly; exiting for supervisor respawn");
      return 1;
    } catch (err) {
      if (deps.isShuttingDown()) {
        log.info(`agent stream aborted during shutdown: ${(err as Error).message}`);
        return 0;
      }
      // A resume against a wiped/missing session errors before any message.
      // One retry without resume (design §3), then give up to the supervisor.
      if (resume !== undefined && !sawMessage && attempt === 1) {
        log.warn(`resume of session ${resume} failed (${(err as Error).message}); retrying without resume`);
        // T8: revert the ledger before requeuing leases, so the retry
        // re-delivers and re-echoes the turns the doomed attempt pulled.
        try {
          deps.onTeardown?.();
        } catch (e) {
          log.warn(`onTeardown hook threw: ${(e as Error).message}`);
        }
        consumer.cancel();
        clearSessionId(deps.sessionFile);
        resume = undefined;
        continue;
      }
      // M2: an auth-shaped throw before any result is a credential failure.
      let classified = authClassified;
      if (!classified && !sawResult && deps.classifyAuthThrow) {
        classified = deps.classifyAuthThrow(err);
      }
      if (classified && deps.onAuthFailure) {
        return await deps.onAuthFailure(classified);
      }
      log.error(`agent session failed: ${(err as Error).message}`);
      return 1;
    } finally {
      deps.onQuery?.(null);
    }
  }
}
