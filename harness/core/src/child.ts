/**
 * Spawns and supervises the `hotline run` child process, and owns the JSON-RPC
 * client bound to its stdio.
 *
 * The harness IS the bridge: unlike the Claude Code / OpenCode harnesses where
 * the coding agent's MCP loader spawns hotline, here the host (the pi
 * extension or the claude-sdk harness) spawns it itself, so nothing MCP-shaped
 * is ever configured on the agent side. The child's `runChannel` takes the
 * injected-harness branch: claude/channel stdio transport, permission relay
 * off, inbound envelope pre-rendered into the notification content.
 *
 * Lifecycle:
 *   - start(): spawn, run the MCP handshake, hand the caller a live client via
 *     onReady. On unexpected exit, respawn with capped exponential backoff.
 *   - stop():  intentional teardown (session_shutdown) — kill the child and do
 *     not respawn.
 *
 * Two exits are fatal (no respawn, loud message):
 *   - binary missing (ENOENT): print the install instructions.
 *   - ownership/poller conflict: another hotline runtime already owns the box
 *     or provider consumer slot — a second session must not silently fight
 *     the first.
 */

import { spawn, type ChildProcess } from "node:child_process";
import {
  JsonRpcClient,
  mcpInitialize,
  mcpListTools,
  timeoutsFromEnv,
  type InitializeResult,
  type McpTool,
  type RpcTimeouts,
} from "./jsonrpc.js";
import { type Logger, defaultLog } from "./log.js";

export interface ChildManagerOptions {
  binary: string;
  args: string[];
  env: NodeJS.ProcessEnv;
  /** clientInfo.name for the MCP handshake (e.g. "hotline-pi", "hotline-claude-sdk"). */
  clientName: string;
  /** Handshake/call deadlines; usually timeoutsFromEnv("<HARNESS PREFIX>"). */
  timeouts?: RpcTimeouts;
  /**
   * Harness README pointer appended to the binary-missing fatal message
   * (e.g. "See the hotline-pi README."). The only wording that differs
   * between harnesses.
   */
  installHint?: string;
  /** Harness-tagged logger; falls back to the stderr-only core logger. */
  log?: Logger;
  onNotification: (method: string, params: unknown) => void;
  /** Called after a successful spawn + MCP handshake + tools/list. */
  onReady: (client: JsonRpcClient, init: InitializeResult, tools: McpTool[]) => Promise<void> | void;
  /** Called whenever the live child goes away (for status / UI). */
  onDown: (reason: string) => void;
  /** Called on a fatal, non-recoverable condition (binary missing, slot conflict). */
  onFatal: (message: string) => void;
}

const BACKOFF_BASE_MS = 500;
const BACKOFF_CAP_MS = 30_000;
// A child that stays up at least this long is treated as healthy: its backoff
// counter resets so a later crash restarts quickly.
const HEALTHY_UPTIME_MS = 10_000;

// Stable stderr markers for box ownership and legacy/provider consumer
// conflicts. Either is fatal: respawning cannot make progress while the other
// runtime holds its lifetime flock.
const ACTIVE_CONFLICT_MARKERS = ["state ownership conflict", "claiming poller slot", "poller slot"];

export function isActiveConflict(stderr: string): boolean {
  const text = stderr.toLowerCase();
  return ACTIVE_CONFLICT_MARKERS.some((marker) => text.includes(marker));
}

/**
 * How to treat a spawn/`error`-event failure (review H2):
 *   - "missing"   → binary absent (ENOENT): fatal, print install instructions.
 *   - "denied"    → EACCES/EPERM: fatal, the file exists but can't be executed.
 *   - "transient" → anything else (EMFILE, EAGAIN, ENOMEM, …): recoverable, so
 *                   schedule a backed-off respawn like an unexpected exit.
 * Node emits ONLY an `error` event for a failed spawn (never `exit`), so a
 * transient spawn error that did not schedule a respawn would strand the bridge
 * forever — hence the explicit respawn path below.
 */
export type SpawnErrorClass = "missing" | "denied" | "transient";
export function classifySpawnError(code: string | undefined): SpawnErrorClass {
  if (code === "ENOENT") return "missing";
  if (code === "EACCES" || code === "EPERM") return "denied";
  return "transient";
}

export class ChildManager {
  private opts: ChildManagerOptions;
  private log: Logger;
  private timeouts: RpcTimeouts;
  private child: ChildProcess | null = null;
  private client: JsonRpcClient | null = null;
  private backoff = BACKOFF_BASE_MS;
  private stopping = false;
  private fatal = false;
  private restartTimer: NodeJS.Timeout | null = null;
  private stderrTail = "";

  constructor(opts: ChildManagerOptions) {
    this.opts = opts;
    this.log = opts.log ?? defaultLog;
    this.timeouts = opts.timeouts ?? timeoutsFromEnv("HOTLINE");
  }

  /** The live client, or null while the child is down. */
  getClient(): JsonRpcClient | null {
    return this.client;
  }

  isFatal(): boolean {
    return this.fatal;
  }

  start(): void {
    if (this.stopping || this.fatal) return;
    this.spawnChild();
  }

  /** Intentional teardown: never respawns. Idempotent. */
  stop(): void {
    this.stopping = true;
    if (this.restartTimer) {
      clearTimeout(this.restartTimer);
      this.restartTimer = null;
    }
    if (this.client) {
      this.client.close("shutdown");
      this.client = null;
    }
    if (this.child) {
      const c = this.child;
      this.child = null;
      try {
        c.kill("SIGTERM");
      } catch {
        // already gone
      }
    }
  }

  private spawnChild(): void {
    this.log.info(`spawning ${this.opts.binary} ${this.opts.args.join(" ")}`);
    // One spawn attempt settles exactly once: `error` and `exit` can both fire
    // for the same child, and both must not schedule a respawn (review H2).
    let settled = false;
    const settle = (): boolean => {
      if (settled) return false;
      settled = true;
      return true;
    };

    let child: ChildProcess;
    try {
      child = spawn(this.opts.binary, this.opts.args, {
        stdio: ["pipe", "pipe", "pipe"],
        env: this.opts.env,
      });
    } catch (err) {
      if (settle()) this.handleSpawnError(err as NodeJS.ErrnoException);
      return;
    }
    this.child = child;
    this.stderrTail = "";
    const spawnedAt = Date.now();

    const client = new JsonRpcClient({
      out: {
        write: (chunk) => {
          if (child.stdin && child.stdin.writable) child.stdin.write(chunk);
        },
      },
      onNotification: this.opts.onNotification,
      log: this.log,
    });

    child.stdout?.setEncoding("utf8");
    child.stdout?.on("data", (chunk: string) => client.feed(chunk));

    // The child logs to stderr; re-emit to OUR stderr (never stdout) and keep a
    // small tail so we can classify the exit reason.
    child.stderr?.setEncoding("utf8");
    child.stderr?.on("data", (chunk: string) => {
      process.stderr.write(chunk);
      this.stderrTail = (this.stderrTail + chunk).slice(-4096);
    });

    child.on("error", (err) => {
      if (settle()) this.handleSpawnError(err as NodeJS.ErrnoException);
    });

    child.on("exit", (code, signal) => {
      const uptime = Date.now() - spawnedAt;
      client.close(`child exited (code=${code}, signal=${signal})`);
      if (this.client === client) this.client = null;
      if (this.child === child) this.child = null;

      if (!settle()) {
        // A spawn `error` already handled this attempt (fatal or respawn).
        return;
      }

      if (this.stopping) {
        this.log.info(`child exited during shutdown (code=${code}, signal=${signal})`);
        return;
      }

      this.opts.onDown(`child exited (code=${code}, signal=${signal})`);

      if (this.hasActiveConflict()) {
        this.declareFatal(
          "hotline box/channel already active in another session — this harness cannot start a second runtime for the same box. Close the other session (or its `hotline up`), choose another --bot, or set HOTLINE_STATE_DIR to a separate state directory, then reload.",
        );
        return;
      }

      this.scheduleRespawn(uptime, `child exited (code=${code}, signal=${signal})`);
    });

    // Bind the client now so tool calls can reach the child, then run the
    // handshake. If the handshake fails, the exit handler above will respawn.
    this.client = client;
    void this.handshake(client);
  }

  private async handshake(client: JsonRpcClient): Promise<void> {
    try {
      const init = await mcpInitialize(client, {
        clientName: this.opts.clientName,
        timeoutMs: this.timeouts.handshakeTimeoutMs,
      });
      const tools = await mcpListTools(client, this.timeouts.handshakeTimeoutMs);
      this.log.info(
        `connected to hotline ${init.serverInfo?.version ?? "?"} — ${tools.length} tool(s): ${tools
          .map((t) => t.name)
          .join(", ")}`,
      );
      await this.opts.onReady(client, init, tools);
    } catch (err) {
      // A handshake failure on a live child: kill it so the exit path decides
      // whether to respawn (or classify a slot conflict from stderr).
      this.log.error(`handshake failed: ${(err as Error).message}`);
      if (this.child) {
        try {
          this.child.kill("SIGTERM");
        } catch {
          // already gone
        }
      }
    }
  }

  private hasActiveConflict(): boolean {
    return isActiveConflict(this.stderrTail);
  }

  private handleSpawnError(err: NodeJS.ErrnoException): void {
    // Drop the (now-dead) child/client references so a transient respawn starts
    // clean and getClient() reports down.
    if (this.child) this.child = null;
    if (this.client) {
      this.client.close(`spawn error: ${err.message}`);
      this.client = null;
    }

    switch (classifySpawnError(err.code)) {
      case "missing":
        this.declareFatal(
          `hotline binary not found on PATH (looked for "${this.opts.binary}"). Install it first:\n` +
            "  brew install 1broseidon/tap/hotline   (or: go install github.com/1broseidon/hotline/cmd/hotline@latest)\n" +
            "then run `hotline setup` (paste your bot token) and `hotline pair`." +
            (this.opts.installHint ? ` ${this.opts.installHint}` : ""),
        );
        return;
      case "denied":
        this.declareFatal(
          `hotline binary at "${this.opts.binary}" is not executable (${err.code}). ` +
            "Fix its permissions (e.g. `chmod +x`) or point HOTLINE_BIN at a runnable binary, then reload.",
        );
        return;
      default:
        // Transient (EMFILE, EAGAIN, …): recover like an unexpected exit.
        this.log.error(`spawn error (${err.code ?? "?"}): ${err.message}; will respawn`);
        this.opts.onDown(`spawn error: ${err.message}`);
        this.scheduleRespawn(0, `spawn error: ${err.message}`);
        return;
    }
  }

  /**
   * Schedule a backed-off respawn. `uptime` is the just-ended child's lifetime
   * (0 for a spawn failure): a healthy-long run resets the backoff so a later
   * crash restarts quickly, a fast crash-loop escalates it. No-ops if we are
   * stopping or already fatal.
   */
  private scheduleRespawn(uptime: number, reason: string): void {
    if (this.stopping || this.fatal) return;
    if (this.restartTimer) return; // a respawn is already pending
    if (uptime >= HEALTHY_UPTIME_MS) this.backoff = BACKOFF_BASE_MS;
    const delay = this.backoff;
    this.backoff = Math.min(this.backoff * 2, BACKOFF_CAP_MS);
    this.log.warn(`child down; respawning in ${delay}ms (${reason})`);
    this.restartTimer = setTimeout(() => {
      this.restartTimer = null;
      this.start();
    }, delay);
  }

  private declareFatal(message: string): void {
    this.fatal = true;
    if (this.restartTimer) {
      clearTimeout(this.restartTimer);
      this.restartTimer = null;
    }
    this.log.error(message);
    this.opts.onFatal(message);
  }
}
