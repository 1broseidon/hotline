/**
 * Hand-rolled JSONL JSON-RPC 2.0 client for the hotline child.
 *
 * There is deliberately NO MCP SDK in this package's dependency tree. The
 * hotline child (`hotline run`) is an MCP server built on the Go SDK, whose
 * stdio transport is newline-delimited JSON (ndjson): every JSON-RPC message is
 * one line terminated by a single `\n`, and the reader is a streaming JSON
 * decoder that tolerates `\n`/`\r\n`. So the entire wire contract we need is:
 *
 *   - write:  JSON.stringify(message) + "\n"
 *   - read:   split incoming bytes on "\n", JSON.parse each non-empty line
 *
 * We speak only the handful of methods hotline needs: `initialize`,
 * `notifications/initialized`, `tools/list`, `tools/call`, and we receive the
 * custom `notifications/claude/channel` push. Everything else is handled
 * defensively (unknown server->client requests get a method-not-found reply so
 * the server never hangs; unknown notifications are logged and dropped).
 *
 * Harness identity and deadlines are parameters: the consuming harness passes
 * its clientInfo name to mcpInitialize and its timeouts (usually via
 * `timeoutsFromEnv("HOTLINE_PI")` / `timeoutsFromEnv("HOTLINE_SDK")`) so the
 * operator envs each harness documented keep working unchanged.
 */

import { type Logger, defaultLog } from "./log.js";

/** A minimal writable sink — the child's stdin. */
export interface Writable {
  write(chunk: string): void;
}

export interface JsonRpcOptions {
  /** The child's stdin (or any line sink). */
  out: Writable;
  /** Called for every inbound JSON-RPC notification (a request with no id). */
  onNotification: (method: string, params: unknown) => void;
  /** Harness-tagged logger; falls back to the stderr-only core logger. */
  log?: Logger;
}

interface Pending {
  resolve: (result: unknown) => void;
  reject: (err: Error) => void;
  method: string;
  timer?: ReturnType<typeof setTimeout>;
}

interface WireMessage {
  jsonrpc?: string;
  id?: number | string | null;
  method?: string;
  params?: unknown;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

export class JsonRpcClient {
  private out: Writable;
  private onNotification: (method: string, params: unknown) => void;
  private log: Logger;
  private buffer = "";
  // JSON-RPC ids MUST start at 1: the Go SDK treats a zero id as an invalid
  // (notification) id, so a request with id 0 would never be answered.
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private closed = false;

  constructor(opts: JsonRpcOptions) {
    this.out = opts.out;
    this.onNotification = opts.onNotification;
    this.log = opts.log ?? defaultLog;
  }

  /**
   * Feed a raw chunk of bytes from the child's stdout. Splits on LF, buffering
   * any trailing partial line until the next chunk. Safe to call with arbitrary
   * chunk boundaries.
   */
  feed(chunk: string): void {
    this.buffer += chunk;
    let idx: number;
    while ((idx = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, idx).replace(/\r$/, "");
      this.buffer = this.buffer.slice(idx + 1);
      if (line.trim() === "") continue;
      this.dispatch(line);
    }
  }

  private dispatch(line: string): void {
    let msg: WireMessage;
    try {
      msg = JSON.parse(line) as WireMessage;
    } catch (err) {
      this.log.error(`dropping unparseable line from child: ${(err as Error).message}`);
      return;
    }

    const hasId = msg.id !== undefined && msg.id !== null;
    const isResponse = hasId && (msg.result !== undefined || msg.error !== undefined) && msg.method === undefined;

    if (isResponse) {
      const id = msg.id as number;
      const p = this.pending.get(id);
      if (!p) {
        this.log.warn(`response for unknown id ${id}`);
        return;
      }
      this.pending.delete(id);
      if (p.timer) clearTimeout(p.timer);
      if (msg.error) {
        p.reject(new Error(`${p.method} failed: ${msg.error.message} (code ${msg.error.code})`));
      } else {
        p.resolve(msg.result);
      }
      return;
    }

    if (msg.method !== undefined) {
      if (hasId) {
        // A server->client request. We implement none, but we must answer so
        // the child never blocks waiting on us.
        this.reply(msg.id as number | string, undefined, {
          code: -32601,
          message: `method not found: ${msg.method}`,
        });
        return;
      }
      // A notification. The handler runs synchronously inside the child's
      // stdout `data` event, so a throw here would otherwise become an uncaught
      // exception that takes down the whole host process (review H3). Contain
      // it: a handler is responsible for its own errors, never the transport.
      try {
        this.onNotification(msg.method, msg.params);
      } catch (err) {
        this.log.error(`notification handler for ${msg.method} threw: ${(err as Error).message}`);
      }
      return;
    }

    this.log.warn(`ignoring malformed JSON-RPC message: ${line.slice(0, 200)}`);
  }

  /**
   * Send a request and await its result. If timeoutMs > 0, the pending promise
   * is rejected with a clear timeout error when no response arrives in time, so
   * a wedged-but-alive child fails the caller (a tool call, the handshake)
   * instead of hanging forever (review M1). The pending entry is removed on
   * timeout, so a late response is simply dropped as "unknown id".
   */
  request<T = unknown>(method: string, params?: unknown, timeoutMs = 0): Promise<T> {
    if (this.closed) return Promise.reject(new Error(`client closed; cannot send ${method}`));
    const id = this.nextId++;
    const p = new Promise<T>((resolve, reject) => {
      const entry: Pending = { resolve: resolve as (r: unknown) => void, reject, method };
      if (timeoutMs > 0) {
        entry.timer = setTimeout(() => {
          if (!this.pending.has(id)) return;
          this.pending.delete(id);
          reject(new Error(`${method} timed out after ${timeoutMs}ms (no response from hotline child)`));
        }, timeoutMs);
        // Do not keep the event loop alive solely for this watchdog.
        entry.timer.unref?.();
      }
      this.pending.set(id, entry);
    });
    this.send({ jsonrpc: "2.0", id, method, params });
    return p;
  }

  /** Send a notification (no id, no response expected). */
  notify(method: string, params?: unknown): void {
    if (this.closed) return;
    this.send({ jsonrpc: "2.0", method, params });
  }

  private reply(
    id: number | string,
    result: unknown,
    error?: { code: number; message: string; data?: unknown },
  ): void {
    if (error) this.send({ jsonrpc: "2.0", id, error });
    else this.send({ jsonrpc: "2.0", id, result });
  }

  private send(msg: WireMessage): void {
    // LF-only framing: exactly one message per line, one trailing newline.
    this.out.write(JSON.stringify(msg) + "\n");
  }

  /**
   * Fail all in-flight requests. Called when the child exits so callers unblock
   * instead of hanging forever.
   */
  close(reason: string): void {
    this.closed = true;
    for (const [id, p] of this.pending) {
      if (p.timer) clearTimeout(p.timer);
      p.reject(new Error(`${p.method} aborted: ${reason}`));
      this.pending.delete(id);
    }
  }
}

// ---- MCP method wrappers -------------------------------------------------

/** The subset of the MCP tool shape we consume from tools/list. */
export interface McpTool {
  name: string;
  description?: string;
  inputSchema?: unknown;
}

export interface InitializeResult {
  protocolVersion: string;
  capabilities?: {
    experimental?: Record<string, unknown>;
    [k: string]: unknown;
  };
  serverInfo?: { name?: string; version?: string };
  instructions?: string;
}

// The protocol version we advertise. The Go SDK negotiates: it echoes a version
// it supports or falls back to its latest, so an exact match is not required.
const PROTOCOL_VERSION = "2025-06-18";

/**
 * Request deadlines (review M1). The handshake must complete promptly or the
 * child is wedged and should be torn down; a single tool call may legitimately
 * run longer (e.g. `publish` spins up a tunnel).
 */
export interface RpcTimeouts {
  handshakeTimeoutMs: number;
  callTimeoutMs: number;
}

export const DEFAULT_HANDSHAKE_TIMEOUT_MS = 15_000;
export const DEFAULT_CALL_TIMEOUT_MS = 60_000;

function envMs(env: NodeJS.ProcessEnv, name: string, fallback: number): number {
  const raw = env[name];
  if (!raw) return fallback;
  const n = Number(raw);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

/**
 * Resolve the timeouts from the harness's env family:
 * `<prefix>_HANDSHAKE_TIMEOUT_MS` and `<prefix>_CALL_TIMEOUT_MS` (pi passes
 * "HOTLINE_PI", claude-sdk "HOTLINE_SDK" — the operator envs each harness has
 * always documented). Overridable for operators with slow links; falls back to
 * the defaults on a missing or bad value.
 */
export function timeoutsFromEnv(prefix: string, env: NodeJS.ProcessEnv = process.env): RpcTimeouts {
  return {
    handshakeTimeoutMs: envMs(env, `${prefix}_HANDSHAKE_TIMEOUT_MS`, DEFAULT_HANDSHAKE_TIMEOUT_MS),
    callTimeoutMs: envMs(env, `${prefix}_CALL_TIMEOUT_MS`, DEFAULT_CALL_TIMEOUT_MS),
  };
}

export interface InitializeOptions {
  /** clientInfo.name sent to the child (e.g. "hotline-pi", "hotline-claude-sdk"). */
  clientName: string;
  clientVersion?: string;
  timeoutMs?: number;
}

/**
 * Run the MCP initialize handshake, then send the required
 * `notifications/initialized`. Returns the server's initialize result (which
 * carries the uncapped hotline AgentInstructions in `instructions`).
 */
export async function mcpInitialize(client: JsonRpcClient, opts: InitializeOptions): Promise<InitializeResult> {
  const result = await client.request<InitializeResult>(
    "initialize",
    {
      protocolVersion: PROTOCOL_VERSION,
      capabilities: {},
      clientInfo: { name: opts.clientName, version: opts.clientVersion ?? "0.1.0" },
    },
    opts.timeoutMs ?? DEFAULT_HANDSHAKE_TIMEOUT_MS,
  );
  client.notify("notifications/initialized", {});
  return result;
}

/** List the child's tools. */
export async function mcpListTools(
  client: JsonRpcClient,
  timeoutMs: number = DEFAULT_HANDSHAKE_TIMEOUT_MS,
): Promise<McpTool[]> {
  const res = await client.request<{ tools?: McpTool[] }>("tools/list", {}, timeoutMs);
  return res.tools ?? [];
}

export interface CallToolResult {
  content?: Array<{ type: string; text?: string; [k: string]: unknown }>;
  isError?: boolean;
}

/** Call one tool and return its raw MCP result. */
export function mcpCallTool(
  client: JsonRpcClient,
  name: string,
  args: unknown,
  timeoutMs: number = DEFAULT_CALL_TIMEOUT_MS,
): Promise<CallToolResult> {
  return client.request<CallToolResult>("tools/call", { name, arguments: args }, timeoutMs);
}
