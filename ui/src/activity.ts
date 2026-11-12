import type { SessionInfo, SessionState, TranscriptEvent } from "./generated/contract";
import type { Streaming } from "./tape";

/**
 * The state a session reads as. A turn the core says is open only for
 * subagents it started (`awaitingSubagents`) reads as done: its reply is in
 * the chat, nothing works under the conversation, and the composer is as it
 * is between turns, while the subagents show at the composer's corner.
 */
export function shownState(session: Pick<SessionInfo, "state" | "awaitingSubagents">): SessionState {
	return session.state === "thinking" && session.awaitingSubagents === true ? "ready" : session.state;
}

/**
 * Whether a teammate is working: the one answer the composer (Interrupt, Esc),
 * the band's beat, the transcript's mark and the work pane all take. A session
 * that is `starting` is working, as the composer already read it; a turn held
 * open only for subagents is not (see `shownState`).
 */
export function isWorking(session: Pick<SessionInfo, "state" | "awaitingSubagents">): boolean {
	const state = shownState(session);
	return state === "starting" || state === "thinking";
}

/** The state a thread pane's composer is given for a turn that runs or does not, so it reads `isWorking` too. */
export function workingState(running: boolean): SessionState {
	return running ? "thinking" : "ready";
}

/**
 * The few words a running process's chip says: the command the process is,
 * past the `cd` that got there and the interpreter that runs it, so
 * `cd site && python3 -m http.server 8770` reads "http.server 8770". The
 * whole command is in the chip's card.
 */
export function processLabel(title: string): string {
	const last = title.split(/&&|;/).map((part) => part.trim()).filter((part) => part !== "").pop() ?? "";
	const label = last.replace(/^exec\s+/, "").replace(/^python[\d.]*\s+-m\s+/, "");
	return label === "" ? title : label;
}

/**
 * What a teammate is doing right now, at the grain the mark can show it.
 *
 * A session's state only says a turn is running. The teammate says what it is
 * on with its `activity` tool, a kind and a few words of its own, and the
 * kind moves the mark: thinking, reading, searching, making and talking each
 * have their own movement. Without a declaration the mark may still move by
 * the kind of tool in use, as the protocol labels it, but nothing is said
 * beside it: the words are the teammate's, and the desk never makes them up.
 * Waiting on you, writing the reply and the hang-up stay the desk's own.
 */
export type ActivityPhase = "thinking" | "read" | "search" | "edit" | "execute" | "talking" | "doing" | "blocked" | "writing" | "landed" | "rest";

export type Activity = {
	phase: ActivityPhase;
	/** The words beside the mark: the teammate's own, or the desk's for what is its own to say (waiting on you). Never a path or a command. */
	word: string;
};

/**
 * The composer's bottom row while a teammate works, and for a moment after:
 * the mark, and whether it is going back to sleep.
 */
export type WorkMark = {
	activity: Activity;
	sleeping: boolean;
};

/**
 * Tool kinds, collapsed to what the mark can actually distinguish, for a
 * teammate that has not said what it is on. An ACP agent sends ACP's kinds;
 * Hotline Agent sends its own tool's name, so its built-ins are here by name
 * too, and a server's tool is read by the part after its `server__` prefix
 * (`computer__shell` runs, `ketch__search` searches). A kind not here is
 * still work, and moves the mark plainly.
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

/** The kinds a teammate may say it is on, and the movement each is; one the desk does not know moves the mark plainly. */
const DECLARED: Record<string, ActivityPhase> = {
	thinking: "thinking",
	reading: "read",
	searching: "search",
	making: "edit",
	talking: "talking",
};

function declaredPhase(type: string): ActivityPhase {
	return DECLARED[type] ?? "doing";
}

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
	if (latest.blocked) return { phase: "blocked", word: "Waiting on you" };
	// Writing outranks what the teammate said it was on: once words are on
	// their way, they are the headline.
	// Bubbles still landing to the beat read as writing, whatever produced them.
	if (queued || streaming.some((one) => one.kind === "agent")) return { phase: "writing", word: "" };
	// What the teammate said it is on stands until it says something else or
	// the turn ends.
	if (latest.declared !== undefined) return { phase: declaredPhase(latest.declared.type), word: latest.declared.message ?? "" };
	// The newest call's kind holds until the next call starts. Reads and
	// listings finish in milliseconds and the gaps between them are the agent
	// choosing its next step in the same kind of work; a mark that dropped to
	// Thinking in every gap would show nothing but thinking. `worked` rather
	// than a truthy kind: plenty of agents send a tool call with no kind at
	// all, and those are still work.
	if (latest.worked) return { phase: kindPhase(latest.kind), word: "" };
	return { phase: "thinking", word: "" };
}

type Declared = Extract<TranscriptEvent, { kind: "activity" }>;

/**
 * What this turn says is happening: a permission still waiting, the newest
 * tool call, running or done, and the newest activity the teammate declared.
 * Walked backwards and stopped once both are known or at the turn's start
 * (the previous turn's end, your message, or an answer delivered to the
 * teammate), because the answer is near the end and this runs on every
 * event of a live turn. The calls that declare are not work of their own.
 */
function scan(events: TranscriptEvent[]): { blocked: boolean; worked?: boolean; kind?: string | undefined; declared?: Declared } {
	let found: { blocked: boolean; worked?: boolean; kind?: string | undefined } | null = null;
	let declared: Declared | undefined;
	for (let index = events.length - 1; index >= 0 && (found === null || declared === undefined); index--) {
		const event = events[index]!;
		if (event.kind === "turn" || event.kind === "user" || event.kind === "delivery") break;
		if (event.kind === "activity") declared ??= event;
		if (found !== null) continue;
		if (event.kind === "permission" && event.decision === undefined) found = { blocked: true };
		else if (event.kind === "tool" && !declares(event)) found = { blocked: false, worked: true, kind: event.toolKind };
	}
	return { ...(found ?? { blocked: false }), ...(declared !== undefined ? { declared } : {}) };
}

/**
 * The step a call to `activity` or `outcome` is, known by the group the
 * core's glance gives it: the line it wrote heads or marks the path, so the
 * call itself is not drawn or counted as work.
 */
export function declares(event: Extract<TranscriptEvent, { kind: "tool" }>): boolean {
	return event.said?.key === "activity" || event.said?.key === "outcome";
}
