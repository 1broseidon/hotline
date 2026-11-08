import type { ThreadEnd, ThreadId, TranscriptEvent } from "./generated/contract";
import type { Person } from "./avatars";
import type { LinkEvent } from "./dock";

/** What names a thread a link stands for. */
export const threadOfLink = (link: LinkEvent): ThreadId => ({ kind: link.threadKind, key: link.thread });

/** A run is seconds to many minutes long: say it the way a person would. */
function runWords(ms: number): string {
	const seconds = Math.round(ms / 1000);
	if (seconds < 1) return "under a second";
	if (seconds < 60) return `${seconds} s`;
	const minutes = Math.floor(seconds / 60);
	const rest = seconds % 60;
	return rest === 0 ? `${minutes} min` : `${minutes} min ${rest} s`;
}

/**
 * How a subagent's run has got to, in the words its line ends with: while it
 * works, what it did last when the roster says (`step`), else that it works.
 */
export function runState(link: Pick<LinkEvent, "state" | "end" | "elapsedMs">, step?: string): string {
	if (link.state !== "closed") return step !== undefined && step !== "" ? step : "working";
	const took = link.elapsedMs === undefined ? "" : ` after ${runWords(link.elapsedMs)}`;
	switch (link.end) {
		case "failed":
			return `failed${took}`;
		case "cancelled":
		case "stopped":
			return `stopped${took}`;
		default:
			return link.elapsedMs === undefined ? "done" : `done in ${runWords(link.elapsedMs)}`;
	}
}

/** A call's saved state is its lifetime, not whether a turn is running. */
function callWords(link: LinkEvent, working: boolean): string {
	if (link.state !== "closed") {
		const state = working ? "running" : link.state === "parked" ? "parked" : "open";
		return `${link.title} · ${state}`;
	}
	const minutes = Math.round(((link.at ?? link.ts) - link.ts) / 60_000);
	const length = minutes < 1 ? "under a minute" : `${minutes} min`;
	return [link.title, length, link.outcome].filter((part) => part !== undefined && part !== "").join(" · ");
}

/** The ways a work thread ends that nobody said were done. */
const ENDINGS: Partial<Record<ThreadEnd, string>> = { stopped: "stopped", idle: "archived, idle" };

/** A side thread opened without a task is untitled until its first line names it. */
export const sideTitle = (title: string | undefined): string => (title === undefined || title === "" ? "New side thread" : title);

/**
 * Who a work thread is between, from the tape that holds it: handed to a
 * colleague, on the hands that gave it; from a colleague, on the hands that
 * took it; a side thread when nobody handed it over.
 */
function workWho(link: LinkEvent, owner: string, people?: ReadonlyMap<string, Person>): string {
	const opener = link.openerName !== undefined && link.openerName !== "" ? link.openerName : undefined;
	if (opener === undefined || link.openerId === link.personaId) return "Side thread";
	if (link.personaId === undefined || link.personaId === owner) return `From ${opener}`;
	const taker = people?.get(link.personaId)?.name;
	return taker === undefined ? "Handed over" : `Handed to ${taker}`;
}

/**
 * Whether a link is work a colleague handed to the teammate whose tape this
 * is. That is the two teammates' work: it is listed in the threads pane, and
 * the person's conversation does not carry a line for each.
 */
export function handedIn(link: LinkEvent, owner: string): boolean {
	if (link.threadKind !== "side" || link.openerId === undefined || link.openerId === link.personaId) return false;
	return link.personaId === undefined || link.personaId === owner;
}

/**
 * What colleagues have handed this teammate that it is working on now, from
 * the newest line of each, and who handed it: the one line the person's
 * conversation keeps for all of them. `working` is the roster's word on which
 * of its work threads run a turn or have a line waiting for one: a thread
 * that is only open, as Continue leaves it, is not being worked on. A handoff
 * the core recorded as no longer standing is not one the teammate works on
 * for anybody.
 */
export function handoffsUnderway(events: readonly TranscriptEvent[], owner: string, working: readonly string[]): { count: number; from: string[] } {
	const newest = new Map<string, LinkEvent>();
	for (const event of events) if (event.kind === "link" && handedIn(event, owner)) newest.set(event.id, event);
	const open = [...newest.values()].filter((link) => link.state !== "closed" && link.handoffEnded !== true && working.includes(link.thread));
	const from = [...new Set(open.map((link) => link.openerName ?? "a teammate"))];
	return { count: open.length, from };
}

/** Names as a person lists them: "Poe", "Poe and Mack", "Poe, Mack and Ada". */
export function namesSaid(names: readonly string[]): string {
	if (names.length <= 1) return names[0] ?? "";
	return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/**
 * What a work thread's line says: who it is between and its title; parked,
 * that it is waiting to be picked up; once closed, what came of it in one
 * line, and how it ended when nobody said it was done.
 */
function workWords(link: LinkEvent, owner: string, people?: ReadonlyMap<string, Person>): string {
	const between = workWho(link, owner, people);
	const who = `${between} · ${sideTitle(link.title)}`;
	if (link.state === "live") return between === "Side thread" ? `Started a side thread · ${sideTitle(link.title)}` : who;
	if (link.state === "parked") return `${who} · parked`;
	const ending = link.end === undefined ? undefined : ENDINGS[link.end];
	const outcome = link.outcome !== undefined && link.outcome !== "" ? link.outcome : "archived";
	return `${who} · ${outcome}${ending !== undefined && link.outcome !== undefined && link.outcome !== "" ? ` · ${ending}` : ""}`;
}

/**
 * A thread's line in the conversation that holds it: one quiet sentence,
 * whatever the kind. `owner` is whose tape it is, and `people` names the
 * colleagues a handoff is between. `working` is the roster's live turn state,
 * separate from the link's saved lifetime.
 */
export function linkLine(link: LinkEvent, owner = "", people?: ReadonlyMap<string, Person>, working = false, step?: string): string {
	switch (link.threadKind) {
		case "run":
			return `${link.runKind === "shell" ? "Command" : "Subagent"} · ${link.title} · ${runState(link, step)}`;
		case "call":
			return callWords(link, working);
		case "pair":
			return link.state === "live" ? `Talking with ${link.title}` : `Talked with ${link.title}`;
		default:
			return workWords(link, owner, people);
	}
}

/** Whether a link says something did not go well: its row's dot takes the danger colour. */
export const linkFailed = (link: LinkEvent): boolean => link.state === "closed" && link.end === "failed";

/** How a linked thread stands, as its row's dot: going, done, gone wrong, or at rest. */
type LinkTone = "live" | "done" | "failed" | "quiet";

/**
 * A link as its row draws it: which kind of thread (the glyph and the
 * label: a run is a Command when it is one a harness left running, and a
 * pair is a Pair), its title, what came of it in a few words, and the dot's tone.
 * The same facts `linkLine` says as one sentence, which the row keeps as
 * its accessible name.
 */
export type LinkRowFacts = { kind: "thread" | "subagent" | "command" | "pair" | "call"; title: string; outcome: string; tone: LinkTone };

const sentence = (words: string): string => words.charAt(0).toUpperCase() + words.slice(1);

export function linkRow(link: LinkEvent, owner = "", people?: ReadonlyMap<string, Person>, working = false, step?: string): LinkRowFacts {
	const failed = linkFailed(link);
	switch (link.threadKind) {
		case "run":
			return {
				kind: link.runKind === "shell" ? "command" : "subagent",
				title: link.title,
				outcome: sentence(runState(link, step)),
				tone: link.state !== "closed" ? "live" : failed ? "failed" : link.end === "cancelled" || link.end === "stopped" ? "quiet" : "done",
			};
		case "call": {
			if (link.state !== "closed") {
				const state = working ? "Running" : link.state === "parked" ? "Parked" : "Open";
				return { kind: "call", title: link.title, outcome: state, tone: working ? "live" : "quiet" };
			}
			const minutes = Math.round(((link.at ?? link.ts) - link.ts) / 60_000);
			const length = minutes < 1 ? "Under a minute" : `${minutes} min`;
			const outcome = [length, link.outcome].filter((part) => part !== undefined && part !== "").join(" · ");
			return { kind: "call", title: link.title, outcome, tone: failed ? "failed" : "done" };
		}
		case "pair":
			return {
				kind: "pair",
				title: link.state === "live" ? `Talking with ${link.title}` : `Talked with ${link.title}`,
				outcome: "",
				tone: link.state === "live" ? "live" : "done",
			};
		default: {
			const between = workWho(link, owner, people);
			const who = between === "Side thread" ? [] : [between];
			if (link.state === "live") return { kind: "thread", title: sideTitle(link.title), outcome: who[0] ?? "Started", tone: "live" };
			if (link.state === "parked") return { kind: "thread", title: sideTitle(link.title), outcome: [...who, "Parked"].join(" · "), tone: "quiet" };
			const ending = link.end === undefined ? undefined : ENDINGS[link.end];
			const said = link.outcome !== undefined && link.outcome !== "" ? link.outcome : "Archived";
			const parts = [...who, said, ...(ending !== undefined && said !== "Archived" ? [sentence(ending)] : [])];
			return { kind: "thread", title: sideTitle(link.title), outcome: parts.join(" · "), tone: failed ? "failed" : ending !== undefined || said === "Archived" ? "quiet" : "done" };
		}
	}
}
