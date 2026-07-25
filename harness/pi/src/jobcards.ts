/**
 * FB13 — auto job cards for Pi subagents.
 *
 * The box already posts "job cards" (live start→done status tiles in the app
 * channel) from an in-process registry only the running MCP server can touch. A
 * harness surfaces its subagents to that registry by shelling out to the
 * `hotline job` CLI — one call per lifecycle transition, keyed on a stable
 * `--cookie`. This module bridges Pi's subagent lifecycle onto that CLI.
 *
 * Event source: `@tintinweb/pi-subagents` (a separate user-installed pi
 * extension) emits subagent lifecycle events on the shared `pi.events` bus.
 * Verified against @tintinweb/pi-subagents@0.14.1 (dist/index.js):
 *   - `subagents:started`   {id,type,description}                       (:385)
 *   - `subagents:completed` {id,type,description,result,error,status,…} (:354)
 *   - `subagents:failed`    same shape; fired for status error|stopped|aborted (:351)
 *   - `subagents:compacted` {id,type,description,reason,…}              (:392)
 * There is NO `subagents:progress` event; `compacted` is the only mid-run signal
 * the bus carries, so we route it to `job update` as a light heartbeat/detail.
 *
 * cookie = the subagent record id (`data.id`).
 * batch  = the fan-out group id when one is exposed — but pi-subagents keeps its
 *   group id (`record.groupId` / internal `batch-N`) OFF the lifecycle payloads,
 *   so we fall back to the pi session id, which is the stable per-run key. We
 *   ALWAYS pass `--batch` explicitly: `hotline job done` without `--batch` fails
 *   to resolve the cookie to its rollup and mis-cards (box bug, 2026-07-16).
 *
 * Best-effort by contract: a failed CLI call must never disturb the Pi run. Every
 * spawn is detached with stdio discarded, its `error` swallowed, and its exit
 * code ignored.
 */

import { spawn as nodeSpawn } from "node:child_process";
import { createLog } from "@1broseidon/hotline-harness-core/log";

const defaultLog = createLog("hotline-pi", "HOTLINE_PI_LOG");

/** The subset of child_process.spawn we use — injectable for tests. */
export type SpawnFn = (
  command: string,
  args: string[],
  options: { detached: boolean; stdio: "ignore" },
) => { on(event: "error", cb: (err: unknown) => void): void; unref(): void };

/** Minimal logger surface (matches ./log). */
export interface JobLog {
  info: (msg: string) => void;
  warn: (msg: string) => void;
}

export interface JobCardsDeps {
  /** `hotline` binary to shell out to. */
  binary: string;
  /** Stable per-run key stamped as --batch on every call (the pi session id). */
  batch: string;
  spawn?: SpawnFn;
  log?: JobLog;
}

const TITLE_MAX = 80;

/** pi-subagents lifecycle payloads (only the fields we read). */
interface StartedEvent {
  id?: unknown;
  type?: unknown;
  description?: unknown;
}
interface TerminalEvent {
  id?: unknown;
  type?: unknown;
  description?: unknown;
  status?: unknown;
}
interface CompactedEvent {
  id?: unknown;
  reason?: unknown;
}

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}

/** A sensible card title from a subagent record: description, else type, else a default. */
export function titleFrom(description: unknown, type: unknown): string {
  const desc = str(description).trim();
  const base = desc || str(type).trim() || "subagent";
  if (base.length <= TITLE_MAX) return base;
  return `${base.slice(0, TITLE_MAX - 1).trimEnd()}…`;
}

/**
 * Terminal state for a `subagents:failed` event. pi-subagents fires `failed` for
 * status error | stopped | aborted. A user-initiated stop/abort is a cancellation,
 * not an error — the box CLI accepts ok|err|cancelled, so map accordingly.
 */
export function failedState(status: unknown): "err" | "cancelled" {
  const s = str(status);
  return s === "aborted" || s === "stopped" ? "cancelled" : "err";
}

// Pure argv builders (exported for tests). Each returns the args AFTER the binary.
export function startArgs(cookie: string, batch: string, title: string): string[] {
  return ["job", "start", "--cookie", cookie, "--batch", batch, "--title", title];
}
export function doneArgs(cookie: string, batch: string, state: "ok" | "err" | "cancelled"): string[] {
  return ["job", "done", "--cookie", cookie, "--batch", batch, "--state", state];
}
export function updateArgs(cookie: string, detail: string): string[] {
  return ["job", "update", "--cookie", cookie, "--detail", detail];
}

/**
 * Subscribe the extension to pi-subagents lifecycle events and drive `hotline job`.
 * Returns a disposer that unsubscribes every listener (call from session_shutdown).
 * Wire only on the top-level bot (depth 0): worker processes run in their own pi
 * process with their own bus, so their subagents never reach this bus — no
 * double-carding.
 */
export function wireJobCards(
  pi: { events: { on(channel: string, handler: (data: unknown) => void): () => void } },
  deps: JobCardsDeps,
): () => void {
  const spawn = deps.spawn ?? (nodeSpawn as unknown as SpawnFn);
  const log = deps.log ?? defaultLog;
  const { binary, batch } = deps;

  // Fire-and-forget: detached, output discarded, never throws into the run.
  const run = (args: string[]): void => {
    try {
      const child = spawn(binary, args, { detached: true, stdio: "ignore" });
      child.on("error", (err) => {
        // ENOENT (binary not on PATH) etc. A card is a nicety; the run goes on.
        log.warn(`jobcards: ${binary} ${args[1] ?? ""} failed: ${err instanceof Error ? err.message : String(err)}`);
      });
      child.unref();
    } catch (err) {
      // spawn() can throw synchronously (e.g. bad args); swallow it.
      log.warn(`jobcards: spawn threw: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const unsubs: Array<() => void> = [];
  const on = (channel: string, handler: (data: unknown) => void): void => {
    // Guard the handler itself: an event listener that throws would surface on
    // whatever emitted the event (the other extension). Never let that happen.
    unsubs.push(
      pi.events.on(channel, (data) => {
        try {
          handler(data);
        } catch (err) {
          log.warn(`jobcards: handler for ${channel} threw: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    );
  };

  on("subagents:started", (data) => {
    const e = (data ?? {}) as StartedEvent;
    const cookie = str(e.id);
    if (!cookie) return;
    run(startArgs(cookie, batch, titleFrom(e.description, e.type)));
  });

  on("subagents:completed", (data) => {
    const e = (data ?? {}) as TerminalEvent;
    const cookie = str(e.id);
    if (!cookie) return;
    run(doneArgs(cookie, batch, "ok"));
  });

  on("subagents:failed", (data) => {
    const e = (data ?? {}) as TerminalEvent;
    const cookie = str(e.id);
    if (!cookie) return;
    run(doneArgs(cookie, batch, failedState(e.status)));
  });

  // No true progress event exists on the bus; compaction is the one mid-run
  // signal, and pushing it as a detail doubles as a heartbeat that keeps the
  // card warm against the box-side lease reaper. Optional and best-effort.
  on("subagents:compacted", (data) => {
    const e = (data ?? {}) as CompactedEvent;
    const cookie = str(e.id);
    if (!cookie) return;
    const reason = str(e.reason);
    run(updateArgs(cookie, reason ? `compacting context (${reason})` : "compacting context"));
  });

  log.info(`jobcards: wired to pi.events (batch=${batch}, bin=${binary})`);

  return () => {
    for (const u of unsubs) {
      try {
        u();
      } catch {
        /* ignore */
      }
    }
    unsubs.length = 0;
  };
}
