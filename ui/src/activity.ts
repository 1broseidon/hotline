import type { TranscriptEvent } from "./generated/contract";
import type { Streaming } from "./tape";

/**
 * What a teammate is doing right now, at the grain the mark can show it.
 *
 * A session's state only says a turn is running. That is enough to raise an
 * indicator and not enough to drive one: reaching for a file, hunting for a
 * string, running a command and writing to you are different kinds of work,
 * and the protocol already labels them. Everything here is read off events
 * that actually happened — nothing is inferred from elapsed time or invented
 * to fill a gap, because a mark that moves for reasons you cannot name is
 * decoration.
 */
export type ActivityPhase = "thinking" | "read" | "search" | "edit" | "execute" | "doing" | "blocked" | "writing" | "landed";

export type Activity = {
	phase: ActivityPhase;
	/** One word beside the mark. Never a path or a command. */
	word: string;
};

/**
 * ACP's tool kinds, collapsed to what the mark can actually distinguish.
 * Seven kinds would be seven animations nobody learns. Four physical
 * categories — looking at, looking for, working on, running — are legible
 * at 30px without being taught, because they are what those verbs look like.
 */
const KINDS: Record<string, ActivityPhase> = {
	read: "read",
	search: "search",
	fetch: "search",
	edit: "edit",
	move: "edit",
	delete: "edit",
	execute: "execute",
	think: "thinking",
};

const WORDS: Record<ActivityPhase, string> = {
	thinking: "Thinking",
	read: "Reading",
	search: "Searching",
	edit: "Editing",
	execute: "Running",
	doing: "Working",
	blocked: "Waiting on you",
	writing: "",
	landed: "",
};

/**
 * The reply has landed and the turn is over, but the mark stays a moment to
 * hang up: never read off the tape, only held by the transcript for the
 * glyph's LANDED_MS after a turn that ended while writing.
 */
export const LANDED: Activity = { phase: "landed", word: "" };

/** What the tape says is happening in a turn that is running. */
export function activityOf(events: TranscriptEvent[], streaming: Streaming[], queued = false): Activity {
	const latest = scan(events);
	// Blocked outranks everything: it is the one state where nothing is
	// happening and nothing will until you answer.
	if (latest.blocked) return activity("blocked");
	// Writing outranks the tool that produced it: once words are on their
	// way, what produced them is no longer the headline.
	// Bubbles still landing to the beat read as writing, whatever produced them.
	if (queued || streaming.some((one) => one.kind === "agent")) return activity("writing");
	// `running` rather than a truthy kind: plenty of agents send a tool call
	// with no kind at all, and those are still work.
	if (latest.running) return activity(KINDS[latest.kind ?? ""] ?? "doing");
	return activity("thinking");
}

const activity = (phase: ActivityPhase): Activity => ({ phase, word: WORDS[phase] });

/**
 * The last thing in the transcript that says what is happening. Walked
 * backwards and stopped early, because the answer is always near the end
 * and this runs on every event of a live turn.
 */
function scan(events: TranscriptEvent[]): { blocked: boolean; running?: boolean; kind?: string | undefined } {
	for (let index = events.length - 1; index >= 0; index--) {
		const event = events[index]!;
		if (event.kind === "permission" && event.decision === undefined) return { blocked: true };
		if (event.kind !== "tool") continue;
		if (event.status === "pending" || event.status === "in_progress") {
			return { blocked: false, running: true, kind: event.toolKind };
		}
		// Nothing older than the newest finished call can still be running.
		if (event.status === "completed" || event.status === "failed") break;
	}
	return { blocked: false };
}
