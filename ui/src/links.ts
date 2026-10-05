import type { ThreadEnd, ThreadId } from "./generated/contract";
import type { Person } from "./avatars";
import type { LinkEvent } from "./dock";

/** What names a thread a link stands for. */
export const threadOfLink = (link: LinkEvent): ThreadId => ({ kind: link.threadKind, key: link.thread });

/** A run is seconds to many minutes long: say it the way a person would. */
export function runWords(ms: number): string {
	const seconds = Math.round(ms / 1000);
	if (seconds < 1) return "under a second";
	if (seconds < 60) return `${seconds} s`;
	const minutes = Math.floor(seconds / 60);
	const rest = seconds % 60;
	return rest === 0 ? `${minutes} min` : `${minutes} min ${rest} s`;
}

/** How a subagent's run has got to, in the words its line ends with. */
export function runState(link: Pick<LinkEvent, "state" | "end" | "elapsedMs">): string {
	if (link.state !== "closed") return "working";
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

/** What a call's line says: that it is going, or how long it lasted and how it ended. */
function callWords(link: LinkEvent): string {
	if (link.state !== "closed") return `${link.title} · in progress`;
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
 * colleagues a handoff is between.
 */
export function linkLine(link: LinkEvent, owner = "", people?: ReadonlyMap<string, Person>): string {
	switch (link.threadKind) {
		case "run":
			return `Subagent · ${link.title} · ${runState(link)}`;
		case "call":
			return callWords(link);
		case "pair":
			return link.state === "live" ? `Talking with ${link.title}` : `Talked with ${link.title}`;
		default:
			return workWords(link, owner, people);
	}
}

/** Whether a link says something did not go well: the line takes the warning colour. */
export const linkFailed = (link: LinkEvent): boolean => link.state === "closed" && link.end === "failed";
