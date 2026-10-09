import type { TranscriptEvent } from "./generated/contract";

/**
 * Whether the notice is the failure of the latest turn, with nothing said
 * since: the one turn `session.retry` will run again.
 */
export function failedLast(events: TranscriptEvent[], noticeId: string): boolean {
	const index = events.findIndex((event) => event.id === noticeId);
	if (index < 0) return false;
	const after = events.slice(index + 1);
	if (after.some((event) => ["user", "delivery", "chapter"].includes(event.kind))) return false;
	const turns = after.filter((event) => event.kind === "turn");
	return turns.length === 1 && turns[0]!.kind === "turn" && turns[0]!.stopReason === "failed";
}

