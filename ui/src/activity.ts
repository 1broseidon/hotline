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
export type ActivityPhase = "thinking" | "read" | "search" | "edit" | "execute" | "doing" | "blocked" | "writing" | "landed" | "rest";

export type Activity = {
	phase: ActivityPhase;
	/** One word beside the mark. Never a path or a command. */
	word: string;
};

/**
 * Tool kinds, collapsed to what the mark can actually distinguish.
 * Seven kinds would be seven animations nobody learns. Four physical
 * categories — looking at, looking for, working on, running — are legible
 * at 30px without being taught, because they are what those verbs look like.
 *
 * An ACP agent sends ACP's kinds; Hotline Agent sends its own tool's name,
 * so its built-ins are here by name too, and a server's tool is read by the
 * part after its `server__` prefix (`computer__shell` runs, `ketch__search`
 * searches). A kind not here is still work: it shows as Working.
 */
const KINDS: Record<string, ActivityPhase> = {
	read: "read",
	ls: "read",
	search: "search",
	fetch: "search",
	grep: "search",
	glob: "search",
	web_search: "search",
	edit: "edit",
	write: "edit",
	move: "edit",
	delete: "edit",
	execute: "execute",
	exec: "execute",
	bash: "execute",
	shell: "execute",
	powershell: "execute",
	think: "thinking",
};

/** The phase a tool call's kind stands for. */
function kindPhase(kind: string | undefined): ActivityPhase {
	const name = (kind ?? "").split("__").pop() ?? "";
	return KINDS[name] ?? "doing";
}

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
	rest: "",
};

/**
 * The reply has landed and the turn is over, but the mark stays a moment to
 * hang up: never read off the tape, only held by the transcript for the
 * glyph's LANDED_MS after a turn that ended while writing.
 */
export const LANDED: Activity = { phase: "landed", word: "" };

/**
 * The turn is over and the mark is going back to sleep: the handset settles
 * on the cradle and the toad sinks behind the composer. Held by the
 * transcript, like LANDED, never read off the tape.
 */
export const RESTING: Activity = { phase: "rest", word: "" };

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
	// The newest call's kind holds until the next call starts. Reads and
	// listings finish in milliseconds and the gaps between them are the agent
	// choosing its next step in the same kind of work; a mark that dropped to
	// Thinking in every gap would show nothing but thinking. `worked` rather
	// than a truthy kind: plenty of agents send a tool call with no kind at
	// all, and those are still work.
	if (latest.worked) return activity(kindPhase(latest.kind));
	return activity("thinking");
}

const activity = (phase: ActivityPhase): Activity => ({ phase, word: WORDS[phase] });

/**
 * The last thing in this turn that says what is happening: a permission
 * still waiting, or the newest tool call, running or done. Walked backwards
 * and stopped at the first answer or at the turn's start (the previous
 * turn's end, or your message), because the answer is always near the end
 * and this runs on every event of a live turn.
 */
function scan(events: TranscriptEvent[]): { blocked: boolean; worked?: boolean; kind?: string | undefined } {
	for (let index = events.length - 1; index >= 0; index--) {
		const event = events[index]!;
		if (event.kind === "permission" && event.decision === undefined) return { blocked: true };
		if (event.kind === "tool") return { blocked: false, worked: true, kind: event.toolKind };
		if (event.kind === "turn" || event.kind === "user") break;
	}
	return { blocked: false };
}
