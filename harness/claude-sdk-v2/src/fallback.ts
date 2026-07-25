/**
 * The M1 delivery lane executor (design §3.6): send a fallback reply for an
 * operator turn that ended without a reply call. It runs the ledger's settle
 * plan against the live child client via the same mcpCallTool seam the proxy
 * uses — one `reply` call per uncovered (source, chat_id) group, the joined
 * turn text as a single `text` field (the box chunks >4096 natively).
 *
 * The text goes clean — no "[forwarded by harness]" prefix. The model wrote it
 * as an answer; the tagging lives in telemetry and the log, not in the chat.
 *
 * The contract text (Go profile + the per-turn reminder) deliberately does NOT
 * advertise this net: a model told it has a safety lane leans on it, which
 * corrupts the fallback-rate telemetry the lane exists to collect.
 */

import { JsonRpcClient, mcpCallTool, timeoutsFromEnv } from "@1broseidon/hotline-harness-core/jsonrpc";
import type { FallbackAction } from "./ledger.js";
import type { Logger } from "@1broseidon/hotline-harness-core/log";

const RETRY_DELAY_MS = 5_000;

export interface FallbackDeps {
  /** The live child client, or null while the child is down/respawning. */
  getClient: () => JsonRpcClient | null;
  /** Whether the box is multi-provider — reply's schema only requires `source`
   * then, so we include it only when needed (design §3.6). */
  multiProvider: () => boolean;
  log: Logger;
  /** Injectable timer for tests. */
  sleep?: (ms: number) => Promise<void>;
  /** Called on every fired lane, before the send, for telemetry (the wire
   * counter restamp lives here). */
  onFired?: (a: FallbackAction, bytes: number) => void;
}

function defaultSleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms).unref?.());
}

export interface FallbackExecutor {
  /** Run one settle plan's fallbacks. Never throws. */
  run(fallbacks: FallbackAction[]): Promise<void>;
  /** How many lanes fired this process (the harness_info counter). */
  readonly firedCount: number;
}

export function createFallbackExecutor(deps: FallbackDeps): FallbackExecutor {
  const timeouts = timeoutsFromEnv("HOTLINE_SDK");
  const sleep = deps.sleep ?? defaultSleep;
  let fired = 0;

  async function sendOnce(a: FallbackAction): Promise<boolean> {
    const client = deps.getClient();
    if (!client) return false; // child down; caller decides whether to retry
    const args: Record<string, unknown> = { chat_id: a.chat_id, text: a.text };
    if (a.source && deps.multiProvider()) args.source = a.source;
    try {
      const res = await mcpCallTool(client, "reply", args, timeouts.callTimeoutMs);
      return res.isError !== true;
    } catch {
      // Transport condition (child exiting mid-call). Treat as a failed send.
      return false;
    }
  }

  async function one(a: FallbackAction): Promise<void> {
    const bytes = Buffer.byteLength(a.text, "utf8");
    fired++;
    deps.onFired?.(a, bytes);
    let ok = await sendOnce(a);
    if (!ok) {
      // The documented "retry shortly" condition is a child respawn; give it
      // one 5s retry before declaring the send failed. The unanswered question
      // still replays on the next harness restart, so a lost send is recovered.
      await sleep(RETRY_DELAY_MS);
      ok = await sendOnce(a);
    }
    if (ok) {
      deps.log.info(
        `lane: fallback fired (source=${a.source ?? "-"} chat=${a.chat_id} bytes=${bytes} epoch=${a.epoch} outcome=${a.outcome})`,
      );
    } else {
      deps.log.warn(
        `lane: fallback FAILED (source=${a.source ?? "-"} chat=${a.chat_id} bytes=${bytes} epoch=${a.epoch} outcome=fallback_failed) — question will replay on restart`,
      );
    }
  }

  return {
    async run(fallbacks: FallbackAction[]): Promise<void> {
      for (const a of fallbacks) {
        try {
          await one(a);
        } catch (err) {
          deps.log.warn(`lane: fallback threw: ${err instanceof Error ? err.message : String(err)}`);
        }
      }
    },
    get firedCount(): number {
      return fired;
    },
  };
}
