/**
 * Per-turn contract re-injection (design §2.3). The SDK has no
 * system-prompt-per-turn seam, but UserPromptSubmit.additionalContext rides
 * EVERY turn — the recency that keeps the reply contract alive at turn 400 of a
 * long session, which the one-shot preset append provably did not (observed on a
 * live box, 2026-07-23).
 *
 * This is the single source of the reminder text; a unit test asserts it names
 * mcp__hotline__reply and <channel> so it can't drift from the Go-side profile.
 * Deliberately does NOT advertise the fallback lane (design §2.2 / §11.4): a
 * model told it has a safety net leans on it and corrupts the fallback-rate
 * telemetry M1 exists to collect.
 */
export const CONTRACT_REMINDER =
  "Reminder: nothing you write is visible to anyone unless you call mcp__hotline__reply (chat_id from the <channel> tag). End operator turns with a reply call.";
