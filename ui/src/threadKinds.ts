import type { RunKind, ThreadKind } from "./generated/contract";

/**
 * What a thread is, as its pane's head says it: a glyph and a word for the
 * kind, and a plain fact beside it where one is needed (Read only, who
 * started a command); empty where the composer already says it. Threads the
 * person is in (a work thread, a call) stay a conversation; threads they
 * watch (a subagent, a handoff, a pair, a command, a job) show their work as
 * it happens, and `watching` is which family it is.
 */
export type PaneKind = {
	/** A mark for the kind; none where the word says it all (a work thread). */
	glyph: string;
	word: string;
	family: string;
	watching: boolean;
};

const READ_ONLY = "Read only";

/**
 * The kind label and family of a thread. `opener` is the name of the
 * teammate whose hands it came from: on a work thread it makes it a
 * handoff, which the person can step in to; on a shell run it is the
 * teammate who left the command running. `run` says what a run is, a
 * subagent unless its roster row says it is a command.
 */
export function threadFamily(kind: ThreadKind, opener?: string, run: RunKind = "subagent"): PaneKind {
	switch (kind) {
		case "side":
			return opener === undefined
				? { glyph: "", word: "Work thread", family: "", watching: false }
				: { glyph: "⇄", word: "Handoff", family: "", watching: true };
		case "run":
			return run === "shell"
				? { glyph: "$", word: "Command", family: opener === undefined ? "" : `Started by ${opener}`, watching: true }
				: { glyph: "◆", word: "Subagent", family: READ_ONLY, watching: true };
		case "pair":
			return { glyph: "◎", word: "Pair", family: READ_ONLY, watching: true };
		case "call":
			return { glyph: "☏", word: "Call", family: "", watching: false };
		case "job":
			return { glyph: "↻", word: "Job", family: "", watching: true };
		case "dm":
			return { glyph: "", word: "Conversation", family: "", watching: false };
	}
}

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const day = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

/** When a thread started, for its pane's meta line: the time, with the date before it when it was not today. */
export function startedWords(at: number, now: number = Date.now()): string {
	const when = new Date(at);
	const today = when.toDateString() === new Date(now).toDateString();
	return `started ${today ? clock.format(when) : `${day.format(when)}, ${clock.format(when)}`}`;
}
