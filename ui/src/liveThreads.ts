import type { RosterEntry, RunningSide, RunningSubagent, ThreadId, TranscriptEvent } from "./generated/contract";
import { stopwatch } from "./dock";
import { sideTitle } from "./links";
import { type ListKind, markOf } from "./threadList";

/*
 * The threads live in a teammate's session, as the composer counts and lists
 * them (task-437): its work threads, handed over or not, and the subagents and
 * background commands it has running. The phone counts the same two lists off
 * the same roster row (hotline-mobile `threadCount` in src/now.ts), so a desk
 * and a phone looking at one session say one number.
 */

/** How many threads are live in the session: the roster's work threads and runs. */
export function liveThreadCount(entry: Pick<RosterEntry, "sides" | "subagents">): number {
	return (entry.sides?.length ?? 0) + (entry.subagents?.length ?? 0);
}

/** The control's words, held to two figures as the phone holds them: "2 threads", "9+ threads". */
export function threadsWords(count: number): string {
	return `${count > 9 ? "9+" : String(count)} ${count === 1 ? "thread" : "threads"}`;
}

/** What it says to a screen reader, and the panel's title: "2 threads running". */
export function threadsLabel(count: number): string {
	return `${count} ${count === 1 ? "thread" : "threads"} running`;
}

/** One live thread, as a row of the composer's panel. */
export type LiveThread = {
	/** `side:s1`, `run:r1`, as the Threads list keys its rows. */
	id: string;
	thread: ThreadId;
	kind: Extract<ListKind, "work" | "handoff" | "subagent" | "command">;
	title: string;
	/** What it is: "Subagent", "Command", "Work thread", "Handoff from Mack". */
	what: string;
	/** What it is doing now, as far as the roster says. */
	doing: string;
	startedAt: number;
	/** A turn of it is running: the dot beats. */
	working: boolean;
};

/** Who opened each work thread, when a teammate did, as the conversation's links and side lines say. */
function sideOpeners(events: readonly TranscriptEvent[]): Map<string, { personaId: string; name: string }> {
	const found = new Map<string, { personaId: string; name: string }>();
	for (const event of events) {
		if (event.kind === "link" && event.threadKind === "side" && event.openerId !== undefined) {
			found.set(event.thread, { personaId: event.openerId, name: event.openerName ?? "a teammate" });
		} else if (event.kind === "side" && event.openedBy !== undefined) {
			found.set(event.sideId, event.openedBy);
		}
	}
	return found;
}

/**
 * The live threads, the same ones `liveThreadCount` counts: the work threads
 * first, then the runs, each oldest first as the roster lists them.
 */
export function liveThreads(entry: Pick<RosterEntry, "persona" | "sides" | "subagents">, events: readonly TranscriptEvent[]): LiveThread[] {
	const openers = sideOpeners(events);
	return [
		...(entry.sides ?? []).map((side) => sideRow(side, openers.get(side.sideId), entry.persona.id)),
		...(entry.subagents ?? []).map(runRow),
	];
}

function sideRow(side: RunningSide, opener: { personaId: string; name: string } | undefined, self: string): LiveThread {
	const handoff = opener !== undefined && opener.personaId !== self;
	return {
		id: `side:${side.sideId}`,
		thread: { kind: "side", key: side.sideId },
		kind: handoff ? "handoff" : "work",
		title: sideTitle(side.title),
		what: handoff ? `Handoff from ${opener.name}` : markOf("work").word,
		// The roster says whether a turn runs, not what was said: the Threads list's words for the two.
		doing: side.working ? "Working" : "Open",
		startedAt: side.startedAt,
		working: side.working,
	};
}

function runRow(run: RunningSubagent): LiveThread {
	const command = (run.kind ?? "subagent") === "shell";
	const kind = command ? "command" : "subagent";
	const step = run.lastStep !== undefined && run.lastStep !== "" ? run.lastStep : undefined;
	return {
		id: `run:${run.runId}`,
		thread: { kind: "run", key: run.runId },
		kind,
		title: run.title.trim() || markOf(kind).word,
		what: markOf(kind).word,
		// As the Threads list says a run's line: its latest step, else what it runs, else that it is starting.
		doing: step ?? (command ? (run.command ?? "Running") : "Starting"),
		startedAt: run.startedAt,
		working: true,
	};
}

/** How long a thread has been live: 0:07, 3:41, 1:02:09. */
export const liveFor = (thread: Pick<LiveThread, "startedAt">, now: number): string => stopwatch(now - thread.startedAt);
