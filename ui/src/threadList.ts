import type { Aside, AsideLevel, RunningSubagent, ScheduleEntry, ThreadId, ThreadSummary } from "./generated/contract";
import type { ThreadRef } from "./components/Transcript";
import { fromThePerson, sourceName, sourceThread, sourceWords } from "./asides";
import { endedWords, openerOf, openerWords, rowTitle } from "./dock";
import { firstLine, nextText } from "./room";
import { threadFamily } from "./threadKinds";

/*
 * The right-hand pane's one list, Threads (docs/design/threads-list.html,
 * idea A): every work thread, handoff, subagent, command, pair, call and job
 * of the teammate, and every note (aside) it was left, as rows of one shape:
 * a kind mark, a title, one line (its state, latest step or result) and a
 * time. A note is linked to the thread it came from, and that thread's row
 * counts the notes it left.
 */

export type ListKind = "work" | "handoff" | "subagent" | "command" | "pair" | "call" | "job" | "note";

/** One row of the list, whatever it stands for. */
export type ListItem = {
	/** `side:s1`, `job:j1`, `note:a1`: unique in the list. */
	id: string;
	kind: ListKind;
	title: string;
	/** The one line under the title: state, latest step or result; for a note, where it came from. */
	line: string;
	/** What the row's time says, and what orders the list, newest first. */
	at: number;
	/** A turn of it is running now: the latest step shows and the dot beats. */
	live: boolean;
	/** A card in it waits on the person. */
	waiting: boolean;
	/** It has ended: a closed thread, an ended job. A note is never ended. */
	ended: boolean;
	/** For a note: the person has read it. */
	read: boolean;
	/** For a note: its level, "person" being the one to the person. */
	level?: AsideLevel;
	/** For a thread: how many notes it left the teammate. */
	notes: number;
	/** What a press opens; nothing for a note from outside or another desk. */
	open?: ThreadRef;
	/** The thread itself, and whose agent it is, for a row that answers a card. */
	thread?: ThreadId;
	owner?: string;
};

/** Each kind's mark and word, as the thread panes say them; a work thread, which has no glyph there, is ≡ here. */
export function markOf(kind: ListKind): { glyph: string; word: string } {
	switch (kind) {
		case "work":
			return { glyph: "≡", word: threadFamily("side").word };
		case "note":
			return { glyph: "✎", word: "Note" };
		case "handoff":
			return threadFamily("side", "");
		case "subagent":
			return threadFamily("run");
		case "command":
			return threadFamily("run", undefined, "shell");
		case "pair":
		case "call":
		case "job":
			return threadFamily(kind);
	}
}

type Input = {
	/** The teammate's threads as listed, each running or not as the roster says. */
	rows: ThreadSummary[];
	/** The teammate's runs going now, as the roster says. */
	runs: readonly RunningSubagent[];
	jobs: ScheduleEntry[];
	asides: Aside[];
	/** Whose pane this is. */
	listedFor: string;
	nameOf(personaId: string): string | undefined;
	/** What the person is called, for a note of their own Loop in. */
	personName: string;
	now: number;
};

const keyOf = (thread: ThreadId) => `${thread.kind}:${thread.key}`;

/** Every row of the list, newest first. */
export function listItems(input: Input): ListItem[] {
	const { rows, asides, listedFor, nameOf } = input;
	const notes = new Map<string, number>();
	for (const aside of asides) {
		const from = sourceThread(aside);
		if (from !== undefined) notes.set(keyOf(from), (notes.get(keyOf(from)) ?? 0) + 1);
	}
	const items: ListItem[] = [];
	for (const row of rows) {
		if (row.thread.kind === "dm" || row.thread.kind === "job") continue;
		items.push(threadItem(row, input, notes.get(keyOf(row.thread)) ?? 0));
	}
	// A plain reminder posts to the conversation and has no thread, so no row.
	for (const job of input.jobs.filter((one) => one.reminder !== true)) {
		const row = rows.find((one) => one.thread.kind === "job" && one.thread.key === job.id);
		items.push(jobItem(job, row, input, notes.get(`job:${job.id}`) ?? 0));
	}
	const titles = new Map(items.map((item) => [item.id, item.title]));
	for (const aside of asides) items.push(noteItem(aside, titles, listedFor, nameOf, input.personName));
	return items.sort((a, b) => b.at - a.at);
}

function threadItem(row: ThreadSummary, { runs, listedFor, nameOf }: Input, notes: number): ListItem {
	const run = row.thread.kind === "run" ? runs.find((one) => one.runId === row.thread.key) : undefined;
	const kind = kindOf(row, run);
	const title = rowTitle(row, listedFor, nameOf);
	const ended = row.state === "closed";
	const live = !ended && (row.working || run !== undefined);
	const from = openerWords(row, listedFor, nameOf);
	const said = stateLine(kind, row, run, live);
	return {
		id: keyOf(row.thread),
		kind,
		title,
		line: from === "" ? said : `${from} · ${said}`,
		at: row.updatedAt,
		live,
		waiting: !ended && row.waiting,
		ended,
		read: false,
		notes,
		open: { thread: row.thread, title },
		thread: row.thread,
		owner: row.thread.kind === "pair" ? listedFor : row.personaId,
	};
}

function kindOf(row: ThreadSummary, run: RunningSubagent | undefined): ListKind {
	switch (row.thread.kind) {
		case "side":
			return openerOf(row) === undefined ? "work" : "handoff";
		case "run":
			return (run?.kind ?? "subagent") === "shell" ? "command" : "subagent";
		case "pair":
			return "pair";
		case "call":
			return "call";
		default:
			return "job";
	}
}

/** A thread's line: what it is doing now, or what came of it. */
function stateLine(kind: ListKind, row: ThreadSummary, run: RunningSubagent | undefined, live: boolean): string {
	const said = row.preview ?? "";
	const and = (word: string, rest: string) => (rest === "" ? word : `${word} · ${rest}`);
	if (row.state === "closed") {
		if (kind === "subagent" || kind === "command") return endedWords(row, kind === "command");
		return and(kind === "work" || kind === "handoff" ? "Closed" : "Ended", row.outcome ?? said);
	}
	if (row.waiting) return and("Needs an answer", said);
	if (kind === "subagent" || kind === "command") {
		const step = run?.lastStep;
		return step !== undefined && step !== "" ? step : kind === "command" ? (run?.command ?? "Running") : "Starting";
	}
	if (live) return and("Working", said);
	if (row.state === "parked") return and("Parked", said);
	return said === "" ? "Open" : said;
}

function jobItem(job: ScheduleEntry, row: ThreadSummary | undefined, { now }: Input, notes: number): ListItem {
	const ended = job.endedAt !== undefined;
	const live = !ended && row?.working === true;
	const post = job.lastPost === undefined ? "" : firstLine(job.lastPost.text);
	const state = ended ? "Ended" : live ? "Running" : `Next ${nextText(job.nextAt, now)}`;
	const thread = { kind: "job" as const, key: job.id };
	return {
		id: `job:${job.id}`,
		kind: "job",
		title: job.name,
		line: post === "" ? state : `${state} · ${post}`,
		at: row?.updatedAt ?? job.endedAt ?? job.lastPost?.at ?? job.nextAt,
		live,
		waiting: !ended && row?.waiting === true,
		ended,
		read: false,
		notes,
		open: { thread, title: job.name },
		thread,
		owner: job.personaId,
	};
}

function noteItem(aside: Aside, titles: Map<string, string>, listedFor: string, nameOf: (personaId: string) => string | undefined, personName: string): ListItem {
	const from = sourceThread(aside);
	const title = from === undefined ? undefined : titles.get(keyOf(from));
	// Whose words: the person's own Loop in, a colleague's, or the teammate's own thread's; an outside source by its name.
	const who = fromThePerson(aside) ? personName || "you" : aside.by === undefined ? sourceName(aside) : aside.by === listedFor ? "" : (nameOf(aside.by) ?? "a teammate");
	const where = "thread" in aside.from ? (title ?? sourceWords(aside)) : "";
	const source = [who, where].filter((part) => part !== "").join(" · ");
	return {
		id: `note:${aside.id}`,
		kind: "note",
		title: aside.text,
		line: `from ${source}${aside.level === "person" ? " · to you" : ""}`,
		at: aside.ts,
		live: false,
		waiting: false,
		ended: false,
		read: aside.seen !== undefined,
		level: aside.level,
		notes: 0,
		...(from !== undefined ? { open: title === undefined ? { thread: from } : { thread: from, title } } : {}),
	};
}

/** What the list can be narrowed to. */
export type ListFilter = "all" | "notes";

export function filtered(items: ListItem[], filter: ListFilter): ListItem[] {
	return filter === "notes" ? items.filter((item) => item.kind === "note") : items;
}

/*
 * The filter last used, per teammate, for as long as the window is open:
 * what the list opens on, and what the teammate menu's Notes sets.
 */
const filters = new Map<string, ListFilter>();
const watchers = new Set<() => void>();

export function filterOf(personaId: string): ListFilter {
	return filters.get(personaId) ?? "all";
}

export function rememberFilter(personaId: string, filter: ListFilter): void {
	filters.set(personaId, filter);
	for (const watcher of watchers) watcher();
}

export function watchFilters(watcher: () => void): () => void {
	watchers.add(watcher);
	return () => void watchers.delete(watcher);
}
