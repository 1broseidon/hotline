import type { Ability, LinkState, RosterEntry, ScheduleEntry, ThreadId, ThreadKind, ThreadSummary, TranscriptEvent } from "./generated/contract";
import { sideTitle } from "./links";

/**
 * The right-hand pane: side threads for the open teammate, and settings, one
 * at a time. Like the rail, its width is yours to drag, remembered by this
 * window, and put back to the default by a double-click on its edge.
 */
export const DOCK_WIDTH = 360;
export const DOCK_MIN = 300;
export const DOCK_MAX = 560;
export const DOCK_STEP = 16;
export const DOCK_KEY = "hotline.dock.size";
/** The narrowest the conversation is left when the pane stands beside it. */
const CONVERSATION_MIN = 396;

export const clampDock = (width: number): number => Math.round(Math.min(DOCK_MAX, Math.max(DOCK_MIN, width)));

/** The remembered width, or the default when there is none or it is unreadable. */
export function loadDockWidth(storage: Pick<Storage, "getItem"> | undefined = safeStorage()): number {
	try {
		const parsed: unknown = JSON.parse(storage?.getItem(DOCK_KEY) ?? "null");
		if (typeof parsed === "object" && parsed !== null) {
			const { width } = parsed as { width?: unknown };
			if (typeof width === "number" && Number.isFinite(width)) return clampDock(width);
		}
	} catch {
		// Unreadable: the default.
	}
	return DOCK_WIDTH;
}

export function saveDockWidth(width: number, storage: Pick<Storage, "setItem"> | undefined = safeStorage()): void {
	try {
		storage?.setItem(DOCK_KEY, JSON.stringify({ width: clampDock(width) }));
	} catch {
		// Quota, private mode: the next launch opens at the default.
	}
}

function safeStorage(): Storage | undefined {
	try {
		return typeof localStorage === "undefined" ? undefined : localStorage;
	} catch {
		return undefined;
	}
}

/** Where a drag leaves the pane: its edge is on the left, so moving left widens it. */
export const draggedDock = (began: number, startX: number, x: number): number => clampDock(began + startX - x);

/**
 * Whether the pane has to lie over the conversation: beside it, the
 * conversation would be left narrower than it can be read at.
 */
export const dockOverlays = (mainWidth: number, dockWidth: number): boolean => mainWidth < dockWidth + 8 + CONVERSATION_MIN;

/** What a thread is doing, as the list's dot says it. */
export type ThreadDot = "running" | "waiting" | "parked" | "closed" | "idle";

export function threadDot(row: Pick<ThreadSummary, "state" | "working" | "waiting">): ThreadDot {
	if (row.state === "closed") return "closed";
	if (row.waiting) return "waiting";
	if (row.state === "parked") return "parked";
	return row.working ? "running" : "idle";
}

/** Where a kind sits in the list: work first, then the rest in the order a person reaches for them. */
const KIND_RANK: Record<ThreadKind, number> = { side: 0, pair: 1, run: 2, call: 3, dm: 4, job: 5 };

/** Within a kind, what wants you first: a card waiting, a turn running, a thread open, then one parked. */
const DOT_RANK: Record<ThreadDot, number> = { waiting: 0, running: 1, idle: 2, parked: 3, closed: 4 };

/**
 * The list: threads still open grouped by kind (work threads, then pairs,
 * runs and calls), each kind newest first with what wants you at its head, and
 * the closed ones folded apart, newest first. A teammate's own conversation is
 * the window beside the list, so it is not a row, and a job's thread is a row of
 * the Jobs group, which the schedules view feeds.
 */
export function groupThreads(list: ThreadSummary[]): { open: ThreadSummary[]; closed: ThreadSummary[] } {
	const rows = list.filter((row) => row.thread.kind !== "dm" && row.thread.kind !== "job");
	const newest = (a: ThreadSummary, b: ThreadSummary) => b.updatedAt - a.updatedAt;
	return {
		open: rows
			.filter((row) => row.state !== "closed")
			.sort((a, b) => KIND_RANK[a.thread.kind] - KIND_RANK[b.thread.kind] || DOT_RANK[threadDot(a)] - DOT_RANK[threadDot(b)] || newest(a, b)),
		closed: rows.filter((row) => row.state === "closed").sort(newest),
	};
}

/** A thread's link on its parent's stream, the way the window reads it. */
export type LinkEvent = Extract<TranscriptEvent, { kind: "link" }>;

/** A link as a row, for the moment before the list has been asked again. */
export function rowOfLink(link: LinkEvent, personaId: string): ThreadSummary {
	return {
		thread: { kind: link.threadKind, key: link.thread },
		personaId: link.personaId ?? personaId,
		title: link.title,
		state: link.state,
		...(link.end !== undefined ? { end: link.end } : {}),
		...(link.openerId !== undefined ? { opener: { personaId: link.openerId, name: link.openerName ?? "" } } : {}),
		...(link.handoffEnded === true ? { handoffEnded: true } : {}),
		startedAt: link.ts,
		updatedAt: link.at ?? link.ts,
		working: false,
		waiting: false,
		...(link.outcome !== undefined ? { outcome: link.outcome } : {}),
	};
}

/**
 * The listed rows, brought up to what the links say since: a thread that
 * parked or closed reads so at once, and one the list has not heard of yet is
 * a row of its own until the list is asked again. Links are the parent's own
 * account of its threads, so where they disagree with a list they are newer,
 * unless the list has heard of the thread since: a link older than the row's
 * last word (a copy of a link that was never settled, say) does not undo it.
 */
export function withLinks(list: ThreadSummary[], links: LinkEvent[], personaId: string): ThreadSummary[] {
	const rows = new Map(list.map((row) => [`${row.thread.kind}:${row.thread.key}`, row]));
	for (const link of links) {
		if (link.threadKind === "dm") continue;
		const id = `${link.threadKind}:${link.thread}`;
		const known = rows.get(id);
		if (known === undefined) {
			rows.set(id, rowOfLink(link, personaId));
			continue;
		}
		const ended = link.handoffEnded === true;
		if (known.state === link.state && known.end === link.end && known.title === link.title && (known.handoffEnded === true) === ended) continue;
		if (known.updatedAt > (link.at ?? link.ts)) continue;
		const { end: _end, outcome: _outcome, handoffEnded: _ended, ...rest } = known;
		rows.set(id, {
			...rest,
			state: link.state,
			...(link.end !== undefined ? { end: link.end } : {}),
			...(ended ? { handoffEnded: true } : {}),
			...(link.outcome !== undefined ? { outcome: link.outcome } : known.outcome !== undefined ? { outcome: known.outcome } : {}),
			title: link.title,
			working: link.state === "live" && known.working,
			waiting: link.state !== "closed" && known.waiting,
			updatedAt: Math.max(known.updatedAt, link.at ?? 0),
		});
	}
	return [...rows.values()];
}

/** The kinds a row names, when its title would not: a pair has none, and a run or call is not work. */
const KIND_WORDS: Record<ThreadKind, string> = { dm: "", side: "", pair: "Pair", run: "Subagent", call: "Call", job: "Job" };

/**
 * Whose hands a thread came from, when a teammate other than its own opened
 * it: "from Mack". In the pane of the teammate who handed it over, it says
 * whose hands it went to instead: "to Poe".
 */
export function openerWords(row: ThreadSummary, listedFor: string, nameOf: (personaId: string) => string | undefined): string {
	if (row.opener === undefined || row.opener.personaId === row.personaId) return "";
	if (row.opener.personaId === listedFor) return `to ${nameOf(row.personaId) ?? "a teammate"}`;
	return `from ${row.opener.name}`;
}

/** The line a row opens with: whose hands it passed through, and what kind it is when that is not obvious. */
export function rowOpener(row: ThreadSummary, listedFor: string, nameOf: (personaId: string) => string | undefined): string {
	return [KIND_WORDS[row.thread.kind], openerWords(row, listedFor, nameOf)].filter((part) => part !== "").join(" · ");
}

/** The other teammate in a pair: its key is the two ids, joined. */
export const pairWith = (key: string, selfId: string): string | undefined => key.split("~").find((id) => id !== selfId);

/** What a row is called: its own title, else, for a pair, the teammate it is with. */
export function rowTitle(row: ThreadSummary, selfId: string, nameOf: (personaId: string) => string | undefined): string {
	if (row.title !== undefined && row.title !== "") return row.title;
	if (row.thread.kind === "side") return sideTitle(row.title);
	const other = row.thread.kind === "pair" ? pairWith(row.thread.key, selfId) : undefined;
	return other !== undefined ? `With ${nameOf(other) ?? "a teammate"}` : "Conversation";
}

/** A time the way a list says it: now, 5m, 3h, 2d, then the date. */
export function relativeTime(at: number, now: number = Date.now()): string {
	const seconds = Math.max(0, Math.round((now - at) / 1000));
	if (seconds < 45) return "now";
	const minutes = Math.round(seconds / 60);
	if (minutes < 60) return `${minutes}m`;
	const hours = Math.round(minutes / 60);
	if (hours < 24) return `${hours}h`;
	const days = Math.round(hours / 24);
	if (days < 7) return `${days}d`;
	return new Date(at).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/**
 * What a person can do in a thread, by what it is: only a work thread is
 * spoken in, ended and brought back. One a teammate handed over is spoken in
 * and brought back like any work thread, and only its result ends it: the
 * person never archives it. The desk refuses the rest the same way.
 */
export type Powers = { say: boolean; close: boolean; resume: boolean; stop: boolean };

const READ: Powers = { say: false, close: false, resume: false, stop: false };

export function powersOf(kind: ThreadKind, state: LinkState, handedOver = false): Powers {
	// A job's thread is spoken in by the person, who is the desktop; a closed one is the record of an ended job.
	if (kind === "job") return state === "closed" ? READ : { ...READ, say: true };
	if (kind === "call") return state === "closed" ? READ : { ...READ, say: true, stop: true };
	if (kind !== "side") return READ;
	if (handedOver) return { say: state !== "closed", close: false, resume: state === "closed", stop: state !== "closed" };
	return state === "closed" ? { ...READ, resume: true } : { ...READ, say: true, close: true };
}

/**
 * Whose agent a thread is: the teammate whose pane lists it, except that a
 * handoff is listed in the pane of the teammate who handed it over too, and
 * there it is still the receiver's. A pair is both teammates', read from the
 * side of the pane it is open in.
 */
export function threadOwnerId(row: ThreadSummary | undefined, listedFor: string): string {
	return row === undefined || row.thread.kind === "pair" ? listedFor : row.personaId;
}

/**
 * Whether a turn of a thread is running, as the roster row of the teammate
 * whose agent it is says. The roster hears a turn start and end first; a list
 * is read at a moment, and in the pane of the teammate who handed a thread
 * over it is not read again when the receiver's turn ends. A pair, which no
 * roster row follows, is the list's word.
 */
export function turnRunning(owner: RosterEntry | undefined, thread: ThreadId, row: ThreadSummary | undefined): boolean {
	if (owner === undefined) return row?.working === true;
	switch (thread.kind) {
		case "side":
			return owner.sides?.some((side) => side.sideId === thread.key && side.working) ?? false;
		case "run":
			return owner.subagents?.some((run) => run.runId === thread.key) ?? false;
		case "job":
			return owner.workingJobs?.includes(thread.key) ?? false;
		case "call":
			return owner.workingCalls?.includes(thread.key) ?? false;
		default:
			return row?.working === true;
	}
}

/** The listed rows, each running or not as `turnRunning` says. */
export function withTurns(rows: ThreadSummary[], roster: RosterEntry[], listedFor: string): ThreadSummary[] {
	return rows.map((row) => {
		const owner = roster.find((one) => one.persona.id === threadOwnerId(row, listedFor));
		const working = turnRunning(owner, row.thread, row);
		return working === row.working ? row : { ...row, working };
	});
}

/** The teammate who handed a work thread over, when one did rather than the person. */
export function openerOf(row: ThreadSummary | undefined): { personaId: string; name: string } | undefined {
	if (row === undefined || row.thread.kind !== "side" || row.opener === undefined) return undefined;
	return row.opener.personaId !== row.personaId ? row.opener : undefined;
}

/**
 * The Jobs group: every job that has a thread (a plain reminder posts to the
 * conversation and has none), the ones still running soonest first, then the
 * ended ones, most recently ended first.
 */
export function jobRows(entries: ScheduleEntry[]): ScheduleEntry[] {
	const jobs = entries.filter((entry) => entry.reminder !== true);
	const running = jobs.filter((entry) => entry.endedAt === undefined).sort((a, b) => a.nextAt - b.nextAt);
	const ended = jobs.filter((entry) => entry.endedAt !== undefined).sort((a, b) => (b.endedAt ?? 0) - (a.endedAt ?? 0));
	return [...running, ...ended];
}

const LEFT_OUT: Record<Ability, string> = {
	computer: "the computer",
	folders: "folders outside its working directory",
	mcp_servers: "MCP servers",
};

/**
 * What a job runs without, as a sentence: the parts of its teammate's
 * permission set it was made without, so a narrowed clone reads as one. None
 * left out is no sentence.
 */
export function withoutWords(without: Ability[] | undefined): string {
	const names = (without ?? []).map((ability) => LEFT_OUT[ability]);
	if (names.length === 0) return "";
	const last = names.pop();
	return `Runs without ${names.length === 0 ? last : `${names.join(", ")} and ${last}`}.`;
}
