import type { RunningSubagent, ScheduleEntry, ThreadSummary } from "./generated/contract";
import type { ThreadRef } from "./components/Transcript";
import { stopwatch, withTurns } from "./dock";
import { type ListKind, filtered, listItems, markOf } from "./threadList";
import type { RosterEntry } from "./wire";

/*
 * What runs in a teammate's session, as the composer counts and lists it
 * (task-437; George, 2026-11-11): exactly the rows the teammate's Threads list
 * puts in its Live group, read from the same rows by the same function, so
 * "2 threads running" is true and the two never disagree. A thread is in it
 * while a turn of it runs and no card in it waits on the person: a subagent or
 * a command its harness left running, a work thread (handed over or not), a
 * pair or a call with a turn in flight, a job firing.
 */

/** The control's words, held to two figures as the phone holds them: "2 threads", "9+ threads". */
export function threadsWords(count: number): string {
	return `${count > 9 ? "9+" : String(count)} ${count === 1 ? "thread" : "threads"}`;
}

/** What it says to a screen reader, and the panel's title: "2 threads running". */
export function threadsLabel(count: number): string {
	return `${count} ${count === 1 ? "thread" : "threads"} running`;
}

/** One running thread, as a row of the composer's panel. */
export type LiveThread = {
	/** `side:s1`, `run:r1`: the Threads list's own key for the row. */
	id: string;
	kind: ListKind;
	title: string;
	/** Its kind and the list row's line: "Subagent · Read Dock.tsx", "Handoff from Mack · Working". */
	line: string;
	/** What a press opens, as the list's row opens it. */
	open: ThreadRef;
	/** When it started, where that is how long it has run: not a pair or a job, whose thread outlives each turn. */
	since?: number;
	/** The list row's time otherwise. */
	at: number;
};

/** What the Threads list is read from: its rows as listed, the roster, and the teammate's jobs. */
export type LiveInput = {
	/** `thread.list`, brought up to the conversation's links (`useThreadRows`). */
	listed: ThreadSummary[];
	roster: readonly RosterEntry[];
	/** Whose session it is. */
	entry: Pick<RosterEntry, "persona" | "subagents">;
	jobs: ScheduleEntry[];
	nameOf(personaId: string): string | undefined;
	now: number;
};

const NO_RUNS: RunningSubagent[] = [];

/** The Threads list's Live group for the teammate, newest first, as rows of the panel. */
export function liveThreads({ listed, roster, entry, jobs, nameOf, now }: LiveInput): LiveThread[] {
	const listedFor = entry.persona.id;
	const rows = withTurns(listed, [...roster], listedFor);
	// The list's own rows and its Live chip's rule; asides are never live.
	const items = filtered(listItems({ rows, runs: entry.subagents ?? NO_RUNS, jobs, asides: [], listedFor, nameOf, personName: "", now }), "live");
	return items.flatMap((item) => {
		if (item.open === undefined) return [];
		const word = markOf(item.kind).word;
		// "from Mack · Working" reads on from the kind's word; anything else after a dot.
		const line = /^(from|to) /.test(item.line) ? `${word} ${item.line}` : `${word} · ${item.line}`;
		const row = rows.find((one) => item.thread !== undefined && one.thread.kind === item.thread.kind && one.thread.key === item.thread.key);
		const run = item.thread?.kind === "run" ? entry.subagents?.find((one) => one.runId === item.thread?.key) : undefined;
		const since = item.kind === "pair" || item.kind === "job" ? undefined : (run?.startedAt ?? row?.startedAt);
		return [{ id: item.id, kind: item.kind, title: item.title, line, open: item.open, at: item.at, ...(since !== undefined ? { since } : {}) }];
	});
}

/** How long a thread has run (0:07, 3:41, 1:02:09), where its start says so. */
export const liveFor = (since: number, now: number): string => stopwatch(now - since);
