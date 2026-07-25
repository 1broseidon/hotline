/**
 * Auto job cards for claude-sdk subagents — port of pi's FB13 module
 * (`harness/pi/src/jobcards.ts`) onto the Agent SDK's own task stream.
 *
 * The box posts "job cards" (live start→done status tiles in the app
 * channel) from an in-process registry only the running MCP server can
 * touch. A harness surfaces its subagents to that registry by shelling out
 * to the `hotline job` CLI — one call per lifecycle transition, keyed on a
 * stable `--cookie`.
 *
 * pi has no progress event on its subagent bus and falls back to a
 * best-guess session id for `--batch`. The SDK gives a strictly better event
 * source: `task_started` / `task_updated` system messages already flow
 * through the `for await` loop in agent.ts, each one carrying its own
 * `session_id` — so batch is exact, not a fallback, and no extension bus is
 * needed at all.
 *
 * Mapping (spec workstream 1):
 *   - `task_started` where `subagent_type` is set (a Task-tool subagent — the
 *     pi analogue) and `skip_transcript` is not true and `task_type` is not
 *     `local_workflow` → `job start`. Ambient/housekeeping tasks are
 *     deliberately NOT carded: pi only cards subagent fan-outs, and carding
 *     housekeeping would be *more* than pi does.
 *   - `task_updated` with `patch.status`:
 *       completed → job done --state ok
 *       failed    → job done --state err
 *       killed    → job done --state cancelled   (pi's aborted/stopped analogue)
 *       paused / a `patch.description` change → job update --detail … (mirrors
 *         pi routing `subagents:compacted` to `update` as the only mid-run
 *         heartbeat; paused itself is skipped as noisy, description-change
 *         included).
 *   - cookie = `task_id`; batch = the message's own `session_id`.
 *
 * A per-process Set of started cookies means a `task_updated` for a task we
 * never carded (filtered at start, or from a different attempt) is ignored,
 * and a terminal transition fires `done` at most once — the cookie is
 * dropped from the set the moment its terminal `done` fires.
 *
 * Best-effort by contract, copied verbatim from pi's posture: a failed CLI
 * call must never disturb the run. Every spawn is detached with stdio
 * discarded, its `error` swallowed, and its exit code ignored.
 */

import { spawn as nodeSpawn } from "node:child_process";
import { createLog } from "@1broseidon/hotline-harness-core/log";
import type { SDKMessage } from "@anthropic-ai/claude-agent-sdk";

const defaultLog = createLog("hotline-sdk", "HOTLINE_SDK_LOG");

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
  spawn?: SpawnFn;
  log?: JobLog;
}

const TITLE_MAX = 80;

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}

/** FB13: whether auto job-card reporting is enabled (opt-out via
 * HOTLINE_AUTO_JOBS=0/false/off/no). Copied from pi index.ts:55-58 — same
 * env knob, now shared across harnesses. */
export function autoJobsEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  const v = (env.HOTLINE_AUTO_JOBS ?? "").trim().toLowerCase();
  return v !== "0" && v !== "false" && v !== "off" && v !== "no";
}

/** A sensible card title from a subagent record: description, else
 * subagent_type, else a default. */
export function titleFrom(description: unknown, subagentType: unknown): string {
  const desc = str(description).trim();
  const base = desc || str(subagentType).trim() || "subagent";
  if (base.length <= TITLE_MAX) return base;
  return `${base.slice(0, TITLE_MAX - 1).trimEnd()}…`;
}

/**
 * Terminal state for a `task_updated` patch.status. The SDK's status enum is
 * at least as granular as pi's bus (no gap): `completed` is ok, `failed` is
 * err, `killed` is the aborted/stopped analogue → cancelled. Anything else
 * (pending/running/paused, or an unrecognized value) is not terminal.
 */
export function stateForStatus(status: unknown): "ok" | "err" | "cancelled" | null {
  const s = str(status);
  if (s === "completed") return "ok";
  if (s === "failed") return "err";
  if (s === "killed") return "cancelled";
  return null;
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

/** The task_started/task_updated fields we read. Structural, not imported
 * from the SDK's system-message union, so a message that hasn't narrowed to
 * those subtypes yet can still be probed safely. */
interface TaskStartedLike {
  task_id?: unknown;
  description?: unknown;
  subagent_type?: unknown;
  task_type?: unknown;
  skip_transcript?: unknown;
  session_id?: unknown;
}
interface TaskUpdatedLike {
  task_id?: unknown;
  session_id?: unknown;
  patch?: {
    status?: unknown;
    description?: unknown;
  };
}

export interface JobCards {
  /** Feed every SDK stream message through here; only task_started/
   * task_updated system messages do anything. Never throws. */
  onMessage(msg: SDKMessage): void;
  /** Drop the started-cookie tracking set. Call once, at process end. */
  dispose(): void;
}

/**
 * Build the job-cards bridge. Same detached fire-and-forget spawn contract
 * as pi's wireJobCards, same swallow-everything error posture.
 */
export function createJobCards(deps: JobCardsDeps): JobCards {
  const spawn = deps.spawn ?? (nodeSpawn as unknown as SpawnFn);
  const log = deps.log ?? defaultLog;
  const binary = deps.binary;

  // Cookies we have carded a `start` for and not yet closed out. Membership
  // is the guard against unknown-cookie updates and double `done`s.
  const started = new Set<string>();

  // Fire-and-forget: detached, output discarded, never throws into the run.
  // Copied verbatim from pi's jobcards.ts:127-139.
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

  function handleTaskStarted(e: TaskStartedLike): void {
    const cookie = str(e.task_id);
    if (!cookie) return;
    // Only Task-tool subagents are carded — the pi analogue. Ambient/
    // housekeeping tasks (no subagent_type), transcript-hidden tasks, and
    // local_workflow runs are deliberately skipped: carding them would be
    // MORE than pi does.
    if (!str(e.subagent_type)) return;
    if (e.skip_transcript === true) return;
    if (e.task_type === "local_workflow") return;
    started.add(cookie);
    run(startArgs(cookie, str(e.session_id), titleFrom(e.description, e.subagent_type)));
  }

  function handleTaskUpdated(e: TaskUpdatedLike): void {
    const cookie = str(e.task_id);
    if (!cookie || !started.has(cookie)) return; // unknown/uncarded cookie: ignore
    const state = stateForStatus(e.patch?.status);
    if (state !== null) {
      // Terminal: drop the cookie first so a duplicate/late transition for
      // the same task is treated as unknown and fires at most once.
      started.delete(cookie);
      run(doneArgs(cookie, str(e.session_id), state));
      return;
    }
    // No true progress signal exists beyond status; a description change is
    // the nearest mid-run heartbeat (mirrors pi routing `subagents:compacted`
    // to `update`). `paused` alone is skipped as noisy.
    const desc = e.patch?.description;
    if (typeof desc === "string" && desc.trim() !== "") {
      run(updateArgs(cookie, desc));
    }
  }

  function onMessage(msg: SDKMessage): void {
    // Guard the handler itself: this is called from the agent's stream loop,
    // and a throw here must never take the session down alongside it.
    try {
      const m = msg as { type?: unknown; subtype?: unknown };
      if (m.type !== "system") return;
      if (m.subtype === "task_started") {
        handleTaskStarted(msg as unknown as TaskStartedLike);
      } else if (m.subtype === "task_updated") {
        handleTaskUpdated(msg as unknown as TaskUpdatedLike);
      }
    } catch (err) {
      log.warn(`jobcards: onMessage threw: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  function dispose(): void {
    started.clear();
  }

  log.info(`jobcards: wired (bin=${binary})`);

  return { onMessage, dispose };
}
