/**
 * M2 — auth containment (design §4).
 *
 * Today a bad credential surfaces as a first-turn stream error, runAgent
 * returns 1, and the supervisor respawns forever with the operator never told.
 * authwatch classifies auth-shaped failures, counts consecutive ones across
 * respawns in a state file, notifies the last operator ONCE on the third, and
 * escalates to a cold-loop contract with the supervisor (marker + exit code 5).
 *
 * The state file and marker live in HOTLINE_SUPERVISOR_DIR beside the session
 * file. A successful `init` clears both, which naturally restores normal
 * backoff on the supervisor side.
 *
 * Classification is pure and exported for unit tests; the counter/notify/marker
 * logic takes injectable I/O so it is testable without a real box.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import type { SDKMessage } from "@anthropic-ai/claude-agent-sdk";
import type { Logger } from "@1broseidon/hotline-harness-core/log";

/** File names in the supervisor dir. */
export const AUTH_STATE_FILE = "claude-sdk-auth.json";
export const AUTH_FATAL_MARKER = "auth.fatal";
export const LAST_OPERATOR_FILE = "last-operator.json";

/** Consecutive auth failures before we notify + escalate (design §4.2). */
export const ESCALATE_AT = 3;

/** Exit code the harness uses on an auth-fatal escalation — new, documented
 * beside the existing 0/1/3/4 ladder (design §4.2). */
export const EXIT_AUTH_FATAL = 5;

const AUTH_FATAL_ERRORS = new Set(["authentication_failed", "oauth_org_not_allowed"]);
// billing_error is NOT auth-fatal — it self-heals; classified as null (log only).
const THROW_RE = /401|invalid[_ ]api[_ ]key|OAuth.*(expired|revoked)|authentication/i;

/**
 * Classify a stream message as an auth failure (design §4.1). Returns a short
 * error string when auth-shaped, else null. Structural reads (not typed union
 * narrowing) so a message that hasn't narrowed is still probed safely.
 */
export function classifyAuthMessage(msg: SDKMessage): string | null {
  const m = msg as { type?: unknown; error?: unknown };
  if (m.type === "auth_status") {
    const err = (msg as { error?: unknown }).error;
    if (typeof err === "string" && err !== "") return `auth_status: ${err}`;
    return null; // an auth_status with no error is progress, not a failure
  }
  if (m.type === "assistant") {
    const err = (msg as { error?: unknown }).error;
    if (typeof err === "string" && AUTH_FATAL_ERRORS.has(err)) return err;
  }
  return null;
}

/**
 * Classify a thrown stream error as auth-shaped (design §4.1, belt for SDK
 * versions that throw instead of streaming the status). Only meaningful before
 * any result — the caller gates on that.
 */
export function classifyAuthThrow(err: unknown): string | null {
  const msg = err instanceof Error ? err.message : String(err);
  return THROW_RE.test(msg) ? `stream error: ${msg}` : null;
}

interface AuthState {
  consecutive: number;
  last_ts: string;
  notified: boolean;
}

export interface AuthWatchDeps {
  supervisorDir: string | undefined;
  log: Logger;
  /** Send the one-shot operator notify. Returns true if a message went out.
   * The wiring reads last-operator.json and calls reply on the live child. */
  notify: (errorText: string) => Promise<boolean>;
}

export interface AuthWatch {
  /** Called on a successful `init` stream message: clear the counter + marker,
   * restoring normal supervisor backoff. */
  onInit(): void;
  /** Called once per process when an auth failure is classified. Advances the
   * consecutive counter, escalates at the threshold, and returns the process
   * exit code (1 for a transient blip, EXIT_AUTH_FATAL when cold-looping). */
  onAuthFailure(errorText: string): Promise<number>;
}

function statePath(dir: string): string {
  return path.join(dir, AUTH_STATE_FILE);
}

function readState(dir: string): AuthState {
  try {
    const raw = fs.readFileSync(statePath(dir), "utf8");
    const p = JSON.parse(raw) as Partial<AuthState>;
    return {
      consecutive: typeof p.consecutive === "number" && p.consecutive > 0 ? p.consecutive : 0,
      last_ts: typeof p.last_ts === "string" ? p.last_ts : "",
      notified: p.notified === true,
    };
  } catch {
    return { consecutive: 0, last_ts: "", notified: false };
  }
}

function writeState(dir: string, st: AuthState, log: Logger): void {
  try {
    const tmp = `${statePath(dir)}.tmp`;
    fs.writeFileSync(tmp, JSON.stringify(st) + "\n");
    fs.renameSync(tmp, statePath(dir));
  } catch (err) {
    log.warn(`authwatch: state write failed: ${(err as Error).message}`);
  }
}

export function createAuthWatch(deps: AuthWatchDeps): AuthWatch {
  const dir = deps.supervisorDir;

  function clear(): void {
    if (!dir) return;
    for (const f of [AUTH_STATE_FILE, AUTH_FATAL_MARKER]) {
      try {
        fs.unlinkSync(path.join(dir, f));
      } catch {
        // already gone
      }
    }
  }

  return {
    onInit(): void {
      clear();
    },
    async onAuthFailure(errorText: string): Promise<number> {
      if (!dir) {
        // Unsupervised run: nothing to persist/escalate against. Report the
        // failure and exit non-zero so a wrapper sees it.
        deps.log.error(`authwatch: auth failure (unsupervised, no escalation): ${errorText}`);
        return 1;
      }
      const st = readState(dir);
      const n = st.consecutive + 1;
      const next: AuthState = { consecutive: n, last_ts: new Date().toISOString(), notified: st.notified };
      deps.log.error(`authwatch: auth failure #${n} — ${errorText}`);

      if (n < ESCALATE_AT) {
        // Transient blip (token refresh race, brief API auth outage): exit 1,
        // let the supervisor respawn at normal backoff.
        writeState(dir, next, deps.log);
        return 1;
      }

      // n >= ESCALATE_AT: cold-loop escalation.
      if (!next.notified) {
        try {
          const sent = await deps.notify(errorText);
          next.notified = true; // notify-once, even if the send itself failed
          if (sent) deps.log.info("authwatch: notified operator of credential death (once)");
          else deps.log.warn("authwatch: no operator to notify (no last-operator file); marker written anyway");
        } catch (err) {
          next.notified = true;
          deps.log.warn(`authwatch: notify threw: ${(err as Error).message}`);
        }
      }
      writeState(dir, next, deps.log);
      // Marker: one line, the classified error. Supervisor pins backoff at Max
      // while it exists; the harness deletes it on the next successful init.
      try {
        fs.writeFileSync(path.join(dir, AUTH_FATAL_MARKER), `${errorText}\n`);
      } catch (err) {
        deps.log.warn(`authwatch: marker write failed: ${(err as Error).message}`);
      }
      return EXIT_AUTH_FATAL;
    },
  };
}

/** Persist the last operator (source, chat_id) for the M2 notify target
 * (design §4.2/§11.5). Best-effort; called from the ledger register path. */
export function saveLastOperator(dir: string | undefined, source: string | undefined, chat_id: string): void {
  if (!dir || !chat_id) return;
  try {
    const tmp = path.join(dir, `${LAST_OPERATOR_FILE}.tmp`);
    fs.writeFileSync(tmp, JSON.stringify({ source: source ?? "", chat_id }) + "\n");
    fs.renameSync(tmp, path.join(dir, LAST_OPERATOR_FILE));
  } catch {
    // best-effort; a lost last-operator only costs the notify a target
  }
}

/** Read the last operator (source, chat_id), or null. */
export function loadLastOperator(dir: string | undefined): { source: string; chat_id: string } | null {
  if (!dir) return null;
  try {
    const raw = fs.readFileSync(path.join(dir, LAST_OPERATOR_FILE), "utf8");
    const p = JSON.parse(raw) as { source?: unknown; chat_id?: unknown };
    if (typeof p.chat_id === "string" && p.chat_id !== "") {
      return { source: typeof p.source === "string" ? p.source : "", chat_id: p.chat_id };
    }
  } catch {
    // missing/corrupt
  }
  return null;
}
