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
	/** Routine: done, with nothing in it to read: a finished command, a job whose latest run posted nothing. Active folds it with what is earlier. */
	routine: boolean;
	/** For a note: the person has read it. */
	read: boolean;
	/** For a note: its level, "person" being the one to the person. */
	level?: AsideLevel;
	/** For a thread: how many notes it left the teammate. */
	notes: number;
	/** What search reads, in lower case: the title, the line, what was last said or came of it, a note's words. */
	words: string;
	/** What a press opens; nothing for a note from outside or another desk. */
	open?: ThreadRef;
	/** The thread itself, and whose agent it is, for a row that answers a card. */
	thread?: ThreadId;
	owner?: string;
};

/** Each kind's mark and word, as the thread panes say them. */
export function markOf(kind: ListKind): { glyph: string; word: string } {
	switch (kind) {
		case "work":
			return threadFamily("side");
		case "note":
			return { glyph: "✎", word: "Aside" };
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

/** What search reads of a row. */
const wordsOf = (...parts: (string | undefined)[]): string => parts.filter((part) => part !== undefined).join("\n").toLowerCase();

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
	const line = from === "" ? said : `${from} · ${said}`;
	return {
		id: keyOf(row.thread),
		kind,
		title,
		line,
		at: row.updatedAt,
		live,
		waiting: !ended && row.waiting,
		ended,
		routine: ended && kind === "command",
		read: false,
		notes,
		words: wordsOf(title, line, row.preview, row.outcome),
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
			return (run?.kind ?? row.runKind ?? "subagent") === "shell" ? "command" : "subagent";
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
	const line = post === "" ? state : `${state} · ${post}`;
	return {
		id: `job:${job.id}`,
		kind: "job",
		title: job.name,
		line,
		at: row?.updatedAt ?? job.endedAt ?? job.lastPost?.at ?? job.nextAt,
		live,
		waiting: !ended && row?.waiting === true,
		ended,
		// Its latest run posted nothing to the chat: a run that found nothing to say.
		routine: !live && row?.posted === false,
		read: false,
		notes,
		words: wordsOf(job.name, line, job.prompt, job.lastPost?.text, row?.preview, row?.outcome),
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
	const line = `from ${source}${aside.level === "person" ? " · to you" : ""}`;
	return {
		id: `note:${aside.id}`,
		kind: "note",
		title: aside.text,
		line,
		at: aside.ts,
		live: false,
		waiting: false,
		ended: false,
		routine: false,
		read: aside.seen !== undefined,
		level: aside.level,
		notes: 0,
		words: wordsOf(aside.text, line),
		...(from !== undefined ? { open: title === undefined ? { thread: from } : { thread: from, title } } : {}),
	};
}

/**
 * What the list can be narrowed to. Active, the default, is what wants a
 * look now: what waits on the person, what runs, and what happened today,
 * with the rest folded into one row. A kind's chip is every one of that
 * kind, live and ended; Threads are the ones worked in: work threads,
 * handoffs and commands.
 */
const FILTERS = ["active", "all", "needs", "live", "threads", "subagents", "jobs", "calls", "pairs", "teammates"] as const;
export type ListFilter = (typeof FILTERS)[number];

const KINDS: Partial<Record<ListFilter, readonly ListKind[]>> = {
	threads: ["work", "handoff", "command"],
	subagents: ["subagent"],
	jobs: ["job"],
	calls: ["call"],
	pairs: ["pair"],
	// What other teammates sent this one: the work they handed over and the asides they left.
	teammates: ["handoff", "note"],
};

/** Within a kind's chip: every one, the ones still live, or the ones ended. */
export type Span = "all" | "live" | "ended";

/** Whether a filter is a kind of thread's, which has the Live / Ended switch. */
export const spans = (filter: ListFilter): boolean => filter !== "teammates" && KINDS[filter] !== undefined;

export function filtered(items: ListItem[], filter: ListFilter, span: Span = "all"): ListItem[] {
	const kinds = KINDS[filter];
	if (kinds !== undefined) return items.filter((item) => kinds.includes(item.kind) && (span === "all" || (span === "ended") === item.ended));
	if (filter === "needs") return items.filter((item) => item.waiting);
	if (filter === "live") return items.filter((item) => item.live && !item.waiting);
	return items;
}

/**
 * The rows whose title, line (state, latest step or result), what was last
 * said in them or what they came to, or a note's words, hold every word of
 * the query, whatever the case.
 */
export function searched(items: ListItem[], query: string): ListItem[] {
	const words = query.toLowerCase().split(/\s+/).filter((word) => word !== "");
	if (words.length === 0) return items;
	return items.filter((item) => words.every((word) => item.words.includes(word)));
}

/** The list's groups, in their order, each newest first. */
export type Groups = { needs: ListItem[]; live: ListItem[]; today: ListItem[]; earlier: ListItem[] };

/**
 * Rows into Needs you, Live, Today and Earlier. Under Active, Earlier is
 * "Earlier and routine": what is older than today, and with it what is
 * routine and a note already read, today's or not.
 */
export function grouped(items: ListItem[], now: number, active: boolean): Groups {
	const midnight = new Date(now);
	midnight.setHours(0, 0, 0, 0);
	const groups: Groups = { needs: [], live: [], today: [], earlier: [] };
	for (const item of items) {
		if (item.waiting) groups.needs.push(item);
		else if (item.live) groups.live.push(item);
		else if (item.at >= midnight.getTime() && !(active && (item.routine || item.read))) groups.today.push(item);
		else groups.earlier.push(item);
	}
	return groups;
}

/*
 * The filter last used, per teammate: what the list opens on, across the
 * pane closing and the app restarting, and what the teammate menu's Asides
 * sets. Kept in this window's storage, read where it is asked for.
 */
const FILTER_KEY = "hotline.threads.filter";
const watchers = new Set<() => void>();

function storage(): Storage | undefined {
	try {
		return typeof localStorage === "undefined" ? undefined : localStorage;
	} catch {
		return undefined;
	}
}

function storedFilters(): Record<string, unknown> {
	try {
		const parsed: unknown = JSON.parse(storage()?.getItem(FILTER_KEY) ?? "{}");
		return typeof parsed === "object" && parsed !== null ? (parsed as Record<string, unknown>) : {};
	} catch {
		return {};
	}
}

export function filterOf(personaId: string): ListFilter {
	const stored = storedFilters()[personaId];
	return FILTERS.find((one) => one === stored) ?? "active";
}

export function rememberFilter(personaId: string, filter: ListFilter): void {
	try {
		storage()?.setItem(FILTER_KEY, JSON.stringify({ ...storedFilters(), [personaId]: filter }));
	} catch {
		// Quota, private mode: the list opens on Active next time.
	}
	for (const watcher of watchers) watcher();
}

export function watchFilters(watcher: () => void): () => void {
	watchers.add(watcher);
	return () => void watchers.delete(watcher);
}
