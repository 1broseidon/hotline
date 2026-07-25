/**
 * Inbound delivery: forward one channel envelope to the Pi agent as a user turn.
 *
 * Extracted from index.ts so the retry ladder is unit-testable without a live
 * Pi session (run-unit.mjs Part 5).
 *
 * The hard constraint: this runs INSIDE the child's stdout data handler
 * (jsonrpc.ts feed → onNotification), so it must NEVER throw — an escaped throw
 * would crash the whole pi process. Every path is caught.
 *
 * On Pi 0.80.5 `pi.sendUserMessage` is ASYNC (agent-session.js: it delegates to
 * `async prompt`). When the agent is already streaming and no `deliverAs` is
 * given, `prompt` throws "Agent is already processing…" — but because the
 * function is async, that surfaces as a promise REJECTION, not a synchronous
 * throw (review N1). The guard fires BEFORE anything is enqueued, so retrying
 * that same message as a steer cannot double-deliver.
 *
 * `ctx.isIdle()` is a TOCTOU snapshot: the agent can begin streaming in the same
 * tick a notification arrives, so even an "idle" bare send can lose the race.
 * The ladder therefore handles the already-processing signal on BOTH the async
 * rejection path (the real 0.80.5 behavior) and a synchronous throw (belt and
 * braces for a future/other Pi shape), retrying once as a steer and dropping
 * loudly if that also fails.
 */

/** The subset of the Pi API this module needs. */
export interface DeliverPi {
  sendUserMessage(
    content: string,
    options?: { deliverAs?: "steer" | "followUp" | "nextTurn" },
  ): unknown;
}

export interface DeliverLog {
  warn(msg: string): void;
  error(msg: string): void;
}

// Pi 0.80.5 mid-stream guard message (agent-session.js `prompt`):
// "Agent is already processing. Specify streamingBehavior (…) to queue…".
// Matching it keeps the steer retry scoped to the one race we know is safe.
const ALREADY_PROCESSING = /already processing/i;

// hotline (review F10): this module's whole contract is "never throw". A rejection
// value is not guaranteed to be an Error, so reaching for `.message` blind can
// throw INSIDE a catch handler and produce an unhandled rejection (fatal under
// Node's default --unhandled-rejections=throw). Coerce defensively instead.
function errMsg(err: unknown): string {
  if (err instanceof Error) return err.message;
  const m = (err as { message?: unknown } | null)?.message;
  return typeof m === "string" ? m : String(err);
}

/**
 * Forward `content` to the agent. `streaming` is the caller's isIdle snapshot
 * (true = agent is busy → steer directly). Never throws.
 */
export function deliverToAgent(
  pi: DeliverPi,
  content: string,
  streaming: boolean,
  log: DeliverLog,
): void {
  const sendAsSteer = (): void => {
    try {
      Promise.resolve(pi.sendUserMessage(content, { deliverAs: "steer" })).catch((err: unknown) =>
        log.error(
          `sendUserMessage(steer) rejected; DROPPING inbound message (never delivered): ${errMsg(err)}`,
        ),
      );
    } catch (err) {
      log.error(
        `sendUserMessage(steer) threw; DROPPING inbound message (never delivered): ${errMsg(err)}`,
      );
    }
  };

  if (streaming) {
    sendAsSteer();
    return;
  }

  const retryAsSteer = (how: string, reason: string): void => {
    log.warn(
      `idle sendUserMessage ${how} (agent became busy mid-tick); retrying as steer: ${reason}`,
    );
    sendAsSteer();
  };

  // Idle snapshot: send bare. On 0.80.5 the mid-stream guard rejects the
  // returned promise; the sync catch is belt-and-braces for any Pi shape that
  // throws synchronously instead.
  try {
    Promise.resolve(pi.sendUserMessage(content)).catch((err: unknown) => {
      const msg = errMsg(err);
      if (ALREADY_PROCESSING.test(msg)) {
        retryAsSteer("rejected", msg);
      } else {
        log.error(
          `sendUserMessage failed; DROPPING inbound message (never delivered): ${msg}`,
        );
      }
    });
  } catch (err) {
    retryAsSteer("threw", errMsg(err));
  }
}
