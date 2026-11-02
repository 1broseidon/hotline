import { useEffect, useState } from "react";
import type { Aside, AsideLevel, ThreadId, ThreadKind, TranscriptEvent } from "./generated/contract";
import { wire } from "./wire";

/**
 * Asides: the short lines one of a teammate's threads, or a colleague, left
 * in its peripheral context (`docs/design/thread-awareness.md`, decision 2).
 * The person sees them twice: as a quiet `(·)` line in a conversation, never a
 * bubble, and listed on the teammate's Noted page. Neither is the chat: the
 * receiver's lines are drawn from `aside.list` into its main conversation and
 * are never written there.
 */

/** What a thread of each kind is called on an aside's line. */
const KIND_WORDS: Record<ThreadKind, string> = {
	dm: "main conversation",
	side: "work thread",
	pair: "ask",
	call: "call",
	run: "run",
	job: "job",
};

/** A level as the person reads it: a `person` aside is the one "to you". */
export function levelWord(level: AsideLevel): string {
	return level === "person" ? "to you" : level;
}

/**
 * The thread an aside came from, when it came from one this desk can open:
 * one on a linked desk (`desk` set) is that desk's, and opens nothing here.
 */
export function sourceThread(aside: Aside): ThreadId | undefined {
	return "thread" in aside.from && aside.from.thread.desk === undefined ? aside.from.thread : undefined;
}

/**
 * Where it came from in words: "work thread", or "outside" for an inbound
 * source, whose words came from outside Hotline.
 */
export function sourceWords(aside: Aside): string {
	return "thread" in aside.from ? KIND_WORDS[aside.from.thread.kind] : "outside";
}

/**
 * Whether the person left it: a Loop in… is their own act, so it is from
 * them, though `by` names the teammate whose thread they acted in.
 */
export function fromThePerson(aside: Aside): boolean {
	return aside.loopedIn === true;
}

/** Who left it when no teammate did: the inbound source, by the name it had then. */
export function sourceName(aside: Aside): string {
	return aside.sourceName ?? "An outside source";
}

/** Whether an aside on a thread's own record is one this thread left, rather than one it was left. */
export function leftHere(aside: Aside, personaId: string, here: ThreadId): boolean {
	const from = sourceThread(aside);
	return aside.to !== personaId || (from !== undefined && from.kind === here.kind && from.key === here.key);
}

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const date = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

/** When an aside was left: the time today, the day this week, else the date. */
export function asideTime(at: number, now: number = Date.now()): string {
	const then = new Date(at);
	const today = new Date(now);
	today.setHours(0, 0, 0, 0);
	if (then >= today) return clock.format(then);
	const days = Math.ceil((today.getTime() - then.getTime()) / 86_400_000);
	return days < 7 ? weekday.format(then) : date.format(then);
}

/**
 * The receiver's asides drawn into its main conversation, each at its time,
 * as `aside` lines. With older lines still on the desk, only those since the
 * first one loaded are drawn, so none hangs above a gap. A result's nudge
 * (`result:<request>`) is there to wake the teammate, and the delivery beside
 * it already says the result: where that delivery is drawn, the nudge is
 * not drawn again. Noted still lists it.
 */
export function withAsides(events: TranscriptEvent[], asides: Aside[], more: boolean): TranscriptEvent[] {
	if (asides.length === 0) return events;
	const since = more ? (events[0]?.ts ?? Number.POSITIVE_INFINITY) : Number.NEGATIVE_INFINITY;
	const delivered = new Set<string>();
	for (const event of events) if (event.kind === "delivery" && event.cause.kind === "peer" && event.cause.requestId !== undefined) delivered.add(`result:${event.cause.requestId}`);
	const lines: TranscriptEvent[] = asides
		.filter((aside) => aside.ts >= since && !delivered.has(aside.id))
		.map((aside) => ({ kind: "aside" as const, ...aside }));
	if (lines.length === 0) return events;
	return [...events, ...lines].sort((a, b) => a.ts - b.ts);
}

/**
 * The Noted page's two groups, each newest first: what the person had not
 * seen (`seen` unset, or among `opened`, what was new when the page opened),
 * and the rest.
 */
export function groupNoted(asides: Aside[], opened: ReadonlySet<string> = new Set()): { fresh: Aside[]; earlier: Aside[] } {
	const newest = [...asides].sort((a, b) => b.ts - a.ts);
	const isNew = (aside: Aside) => aside.seen === undefined || opened.has(aside.id);
	return { fresh: newest.filter(isNew), earlier: newest.filter((aside) => !isNew(aside)) };
}

/**
 * A teammate's asides, newest first, read again whenever its roster row's
 * counts move: a new one raises them, and hearing or seeing one lowers them.
 * Undefined until the first read answers.
 */
export function useAsides(personaId: string, counts: { asides?: number | undefined; asidesNew?: number | undefined }): Aside[] | undefined {
	const [asides, setAsides] = useState<Aside[] | undefined>(undefined);
	const signature = `${counts.asides ?? 0}:${counts.asidesNew ?? 0}`;
	// biome-ignore lint/correctness/useExhaustiveDependencies: the counts are what say the list moved.
	useEffect(() => {
		let cancelled = false;
		void wire
			.command("aside.list", { personaId })
			.then((list) => !cancelled && setAsides(list))
			.catch(() => undefined);
		return () => {
			cancelled = true;
		};
	}, [personaId, signature]);
	return asides;
}
