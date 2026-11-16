import type { RunKind, ThreadKind } from "./generated/contract";

/**
 * What a thread is: a glyph for its row in the Threads list and a word for
 * the band of its pane. Threads the person is in (a work thread, a call)
 * stay a conversation; threads they watch (a subagent, a handoff, a pair, a
 * command, a job) show their work as it happens, and `watching` is which
 * family it is.
 */
export type PaneKind = {
	/** A mark for the kind; none where the word says it all (a work thread). */
	glyph: string;
	word: string;
	watching: boolean;
};

/**
 * The kind of a thread. `opener` is the name of the teammate whose hands
 * a work thread came from: it makes it a handoff, which the person can step
 * in to. `run` says what a run is, a subagent unless its roster row says it
 * is a command. A subagent is a
 * "Helper" wherever the person reads it (George, 2026-11-12): the word
 * subagent stays in the code and on the wire.
 */
export function threadFamily(kind: ThreadKind, opener?: string, run: RunKind = "subagent"): PaneKind {
	switch (kind) {
		case "side":
			return opener === undefined
				? { glyph: "≡", word: "Work thread", watching: false }
				: { glyph: "⇄", word: "Handoff", watching: true };
		case "run":
			return run === "shell"
				? { glyph: "$", word: "Command", watching: true }
				: { glyph: "◆", word: "Helper", watching: true };
		case "pair":
			return { glyph: "◎", word: "Pair", watching: true };
		case "call":
			return { glyph: "☏", word: "Call", watching: false };
		case "job":
			return { glyph: "↻", word: "Job", watching: true };
		case "dm":
			return { glyph: "", word: "Conversation", watching: false };
	}
}

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const day = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

/** When something in a pane happened (a thread started, last moved): the time, with the date before it when it was not today. */
export function clockWords(at: number, now: number = Date.now()): string {
	const when = new Date(at);
	const today = when.toDateString() === new Date(now).toDateString();
	return today ? clock.format(when) : `${day.format(when)}, ${clock.format(when)}`;
}
