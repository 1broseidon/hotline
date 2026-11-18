import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import type { InboxItem, InboxItemDetail, InboxNewerItem, InboxOutcome, InboxReceipt, InboxSortedEntry, InboxView, TranscriptEvent } from "./generated/contract";
import { wire } from "./wire";

/*
 * The inbox on the desk (docs/design/agent-inbox.md, "On the desk"): what a
 * teammate sent and where each item stands, how a receiver sorted what it
 * read, and its inbox as the pane lists it: one row per topic, an ask with
 * its answers and follow-ups, which opens as a conversation. Teammates act
 * on items; the person reads them. Nothing here sends, sorts or answers
 * anything, and nothing marks an item seen.
 */

/** An `inbox_sent` line: one item a teammate sent, on the thread it was sent from. */
export type InboxSent = Extract<TranscriptEvent, { kind: "inbox_sent" }>;


/* ------------------------------------------------------------ the wire */

type Watch = { view: InboxView | undefined; listeners: Set<() => void>; stop: () => void };

/** One subscription per teammate's inbox, however many rows, chips and panes read it. */
const watched = new Map<string, Watch>();

function watchInbox(personaId: string, listener: () => void): () => void {
	let watch = watched.get(personaId);
	if (watch === undefined) {
		const made: Watch = { view: undefined, listeners: new Set(), stop: () => {} };
		const tell = () => {
			for (const one of made.listeners) one();
		};
		watched.set(personaId, made);
		// The view is sent whole each time, as the snapshot: what it holds is replaced, never added to.
		made.stop = wire.subscribe<InboxView>(
			{ inbox: personaId },
			{
				snapshot: (view) => {
					made.view = isView(view) ? view : undefined;
					tell();
				},
				event: () => {},
				removed: () => {
					made.view = undefined;
					tell();
				},
			},
		);
		watch = made;
	}
	const kept = watch;
	kept.listeners.add(listener);
	return () => {
		kept.listeners.delete(listener);
		if (kept.listeners.size > 0) return;
		kept.stop();
		if (watched.get(personaId) === kept) watched.delete(personaId);
	};
}

/** What came is a view: a desk that does not keep inboxes answers something else, which is read as none. */
function isView(value: unknown): value is InboxView {
	const view = value as Partial<InboxView> | null;
	return typeof view === "object" && view !== null && Array.isArray(view.open) && Array.isArray(view.closed) && Array.isArray(view.sent);
}

const NOTHING = () => () => {};

/** A teammate's inbox as the desk keeps it now: undefined until the first snapshot lands, or for nobody. */
export function useInbox(personaId: string | undefined): InboxView | undefined {
	// Made once per teammate: a new one each render would end the subscription and start it again.
	const subscribe = useCallback((listener: () => void) => (personaId === undefined ? NOTHING() : watchInbox(personaId, listener)), [personaId]);
	const read = () => (personaId === undefined ? undefined : watched.get(personaId)?.view);
	return useSyncExternalStore(subscribe, read, read);
}

/**
 * One item with its replies (`inbox.item`), read again whenever its
 * receiver's inbox moves: a receipt, its work thread, or a reply it sent.
 */
export function useInboxItem(id: string): { detail: InboxItemDetail | undefined; error: string | null } {
	const [detail, setDetail] = useState<InboxItemDetail | undefined>(undefined);
	const [error, setError] = useState<string | null>(null);
	const view = useInbox(detail?.item.id === id ? detail.item.to : undefined);
	// biome-ignore lint/correctness/useExhaustiveDependencies: the receiver's view moving is what says the item may have.
	useEffect(() => {
		let current = true;
		void wire
			.command("inbox.item", { id })
			.then((next) => {
				if (!current) return;
				setDetail(next);
				setError(null);
			})
			.catch((failed: unknown) => current && setError(failed instanceof Error ? failed.message : String(failed)));
		return () => {
			current = false;
		};
	}, [id, view]);
	return { detail: detail?.item.id === id ? detail : undefined, error };
}

/** Every item a view knows, by id: what the teammate holds, open and closed, and what it sent. */
export function itemsOf(view: InboxView | undefined): ReadonlyMap<string, InboxItem> {
	const items = new Map<string, InboxItem>();
	if (view === undefined) return items;
	for (const item of [...view.sent, ...view.closed, ...view.open]) items.set(item.id, item);
	return items;
}

/**
 * The work threads taking an item started, by key, each with the item taken
 * into it, out of what a view knows: the receiver's own (open and closed) and
 * those its sent items started. The desk opens such a thread with the item's
 * sender as its opener, as a handoff was; this is how the window tells it
 * from a handoff of old, and how the thread's pane opens its message. Items
 * combined into the taken one name the thread too; the one taken, which
 * holds its request, is the thread's message.
 */
export function inboxWork(items: ReadonlyMap<string, InboxItem>): ReadonlyMap<string, string> {
	const work = new Map<string, string>();
	for (const item of items.values()) {
		if (item.work?.kind !== "side") continue;
		if (!work.has(item.work.key) || item.request !== undefined) work.set(item.work.key, item.id);
	}
	return work;
}

/** The same, for a teammate: live from its inbox. */
export function useInboxWork(personaId: string | undefined): ReadonlyMap<string, string> {
	const view = useInbox(personaId);
	return useMemo(() => inboxWork(itemsOf(view)), [view]);
}

/* ----------------------------------------------------------- the words */

/**
 * How an item ended, as one word, the same in the conversation, the inbox
 * pane and the item pane: Answered when a reply went back, Done when its
 * work finished.
 */
export function outcomeWord(outcome: InboxOutcome | undefined): string {
	switch (outcome) {
		case "answered":
			return "Answered";
		case "done":
			return "Done";
		case "noted":
			return "Noted";
		case "dropped":
			return "Dropped";
		case "combined":
			return "Combined";
		case "expired":
			return "Expired";
		default:
			return "Closed";
	}
}

/**
 * Who an item is from, as the desk says it: Hotline for the desk's own
 * notice (an expired item's, marked in its data), else its sender's name.
 */
export function senderWords(item: InboxItem, nameOf: (personaId: string) => string | undefined): string {
	return item.notice !== undefined ? "Hotline" : (nameOf(item.from) ?? "A teammate");
}

/**
 * Where an item stands, as its sender's row and the panes say it. Noticed,
 * the receiver being shown its title, is the desk's own step, so it reads
 * as Sent until the receiver reads it. Taken reads as Picked up wherever the
 * person reads it; the stored state and the teammates' own word stay taken.
 */
export function stateWord(item: InboxItem): string {
	if (item.held) return "Awaiting approval";
	switch (item.state) {
		case "new":
		case "noticed":
			return "Sent";
		case "seen":
			return "Seen";
		case "taken":
			return "Picked up";
		case "later":
			return "Later";
		case "closed":
			return outcomeWord(item.outcome);
	}
}

/** "2 unread", the composer's chip and the menu's count. */
export const unreadWords = (count: number): string => `${count > 9 ? "9+" : String(count)} unread`;

/* ---------------------------------------------------- the sender's rows */

/** Items sent together: one or more `inbox_sent` lines with the same title, side by side on the tape. */
export type SentGroup = { title: string; sent: InboxSent[] };

/** Recipients by name, three at most and the rest counted: "Juno, Ivy, Huck +2". */
export function toWords(ids: readonly string[], nameOf: (personaId: string) => string | undefined): string {
	const names = ids.map((id) => nameOf(id) ?? "a teammate");
	return names.length <= 3 ? names.join(", ") : `${names.slice(0, 3).join(", ")} +${names.length - 3}`;
}

/**
 * Where items sent together stand, from the sender's own view: one item's
 * state word; for several, how many stand at each word, answers first ("1
 * done · 1 seen · 1 awaiting approval", `groupWords`). Nothing while none of
 * them is known yet.
 */
export function sentState(group: SentGroup, items: ReadonlyMap<string, InboxItem>): string | undefined {
	const known = group.sent.flatMap((one) => {
		const item = items.get(one.item);
		return item === undefined ? [] : [item];
	});
	if (known.length === 0) return undefined;
	return group.sent.length === 1 ? stateWord(known[0]!) : groupWords(known);
}

/** The state words in the order a send's words say them: what came back first, then what is still on its way, then what ended without an answer. */
const ANSWERS_FIRST = ["Answered", "Done", "Picked up", "Seen", "Sent", "Later", "Awaiting approval", "Noted", "Combined", "Dropped", "Expired", "Closed"];

/** Where a send to several stands, answers first: "1 done · 1 seen · 1 awaiting approval". */
export function groupWords(items: readonly InboxItem[]): string {
	const counts = new Map<string, number>();
	for (const item of items) counts.set(stateWord(item), (counts.get(stateWord(item)) ?? 0) + 1);
	return [...counts.entries()]
		.sort(([a], [b]) => ANSWERS_FIRST.indexOf(a) - ANSWERS_FIRST.indexOf(b))
		.map(([word, count]) => `${count} ${word.toLowerCase()}`)
		.join(" · ");
}

/* ----------------------------------------------------- the inbox's line */

/** What became of an entry of a sort, in a word, at the right of its row. */
export function sortedWord(entry: InboxSortedEntry): string {
	if (entry.state === "taken") return "Picked up";
	if (entry.state === "later") return "Later";
	if (entry.state === "closed") return outcomeWord(entry.outcome);
	return entry.state.charAt(0).toUpperCase() + entry.state.slice(1);
}

/**
 * What a sort did to one item, after its title, as its row says it: whom it
 * is from (Hotline for the desk's own notice), what became of it in a word,
 * and a later's reason.
 */
export function sortedFacts(entry: InboxSortedEntry, nameOf: (personaId: string) => string | undefined): string {
	const from = `From ${entry.notice !== undefined ? "Hotline" : (nameOf(entry.from) ?? "a teammate")}`;
	return [from, sortedWord(entry), entry.reason].filter((part): part is string => part !== undefined && part !== "").join(" · ");
}

/** One row of a turn's inbox in its path: an item it sorted, as the sort left it, or one it sent. */
export type TurnInboxRow = { kind: "sorted"; key: string; entry: InboxSortedEntry } | { kind: "sent"; key: string; sent: InboxSent };

/**
 * What a turn sorted and sent, in the order it did, read off the tape from
 * the person's line or turn's end before its first step to the line or end
 * after its last: the rows its path lists under Inbox. A quiet turn that
 * posted a bubble has them there and nowhere in the chat (`foldInboxRuns`).
 */
export function turnInbox(events: readonly TranscriptEvent[], stepIds: readonly string[]): TurnInboxRow[] {
	const ids = new Set(stepIds);
	const first = events.findIndex((event) => ids.has(event.id));
	if (first < 0) return [];
	const last = events.findLastIndex((event) => ids.has(event.id));
	const bound = (event: TranscriptEvent) => event.kind === "user" || event.kind === "turn";
	let from = first;
	while (from > 0 && !bound(events[from - 1]!)) from -= 1;
	let to = last;
	while (to + 1 < events.length && !bound(events[to + 1]!)) to += 1;
	return events.slice(from, to + 1).flatMap((event): TurnInboxRow[] => {
		if (event.kind === "inbox_sent") return [{ kind: "sent", key: event.id, sent: event }];
		if (event.kind === "inbox_sorted") return event.entries.map((entry, at) => ({ kind: "sorted", key: `${event.id}:${at}`, entry }));
		return [];
	});
}

/** "Inbox · read 3, sent 1": the line for a run of a teammate's inbox activity, saying only what it has. */
export const runWords = (read: number, sent: number): string =>
	`Inbox · ${[read > 0 && `read ${read}`, sent > 0 && `sent ${sent}`].filter((part): part is string => part !== false).join(", ")}`;

/**
 * The words that seeded a work thread, as its Message shows them to the
 * person: each item's title over its words. The desk hands the teammate a
 * line between them, "From Goldie, item i-linux, wants work:", which the
 * person does not read: the sender is on the Message's rule, and an item
 * combined into it from someone else says "From Quill" alone. Nor does the
 * person read the desk's stand-in for an item with no words. A title keeps
 * its line when the words are read as Markdown, which would run it into them.
 */
export function messageWords(seeded: string): string {
	let first: string | undefined;
	return seeded.replace(/^(.+)\nFrom (.+), item \S+, wants [^:\n]+:\n(\(No message beyond its title\.\)$)?/gm, (_whole, title: string, sender: string, none: string | undefined) => {
		first ??= sender;
		const lines = sender === first ? [title] : [title, `From ${sender}`];
		return none === undefined ? `${lines.join("  \n")}  \n` : lines.join("  \n");
	});
}

/** When an item last moved: its newest receipt, or when it was sent. */
export const movedAt = (item: InboxItem): number => item.receipts.at(-1)?.at ?? item.ts;

/** One line of an item's receipts: when, the state's word, and why where it says; `warn` for what waits on the person. */
export type ReceiptRow = { at: number; word: string; detail?: string; warn?: true };

/** A receipt's state as a word. */
function receiptWord(receipt: InboxReceipt): string {
	switch (receipt.state) {
		case "new":
			return "Allowed";
		case "noticed":
			return "Sent";
		case "seen":
			return "Seen";
		case "taken":
			return "Picked up";
		case "later":
			return "Later";
		case "closed":
			return outcomeWord(receipt.outcome);
	}
}

/**
 * An item's history as the pane's timeline: sent, then every receipt with
 * its time, a later's reason, a close's outcome and why, and what a
 * combined one was folded into. Landing in the inbox is the send itself;
 * landing long after it is the person's first-contact card letting it in.
 * Being noticed is left out: it reads as Sent, which the first row says.
 */
export function receiptRows(item: InboxItem, titleOf: (id: string) => string | undefined = () => undefined): ReceiptRow[] {
	const rows: ReceiptRow[] = [{ at: item.ts, word: "Sent" }];
	for (const receipt of item.receipts) {
		if (receipt.state === "noticed") continue;
		if (receipt.state === "new" && receipt.at - item.ts < LET_IN_AFTER) continue;
		const into = receipt.into === undefined ? undefined : `into ${titleOf(receipt.into) ?? "another message"}`;
		const detail = receipt.reason ?? into;
		rows.push({ at: receipt.at, word: receiptWord(receipt), ...(detail !== undefined && detail !== "" ? { detail } : {}) });
	}
	if (item.held === true && item.state !== "closed") rows.push({ at: item.ts, word: "Awaiting approval", detail: "needs your OK", warn: true });
	return rows;
}

/** A first receipt this long after the send was the first-contact card's yes, not the send. */
const LET_IN_AFTER = 5_000;

/* ------------------------------------------------------------ topics */

/*
 * The pane lists a teammate's inbox as topics (docs/design/agent-inbox.md,
 * "On the desk"): one ask, its answers and the follow-ups on them, between
 * this teammate and whoever it is talking to, read out of its own view,
 * which holds both what it received and what it sent. A send to several is
 * one topic for its sender, each teammate's replies in a lane of their own;
 * the receivers each see only their own part, since their views hold only
 * that.
 */

/** Where a topic sits on Open, or that it is closed. */
export type TopicPlace = "sort" | "picked" | "waiting" | "later" | "closed";

/** The groups of Open, in their order. */
export const TOPIC_GROUPS: readonly { place: TopicPlace; title: string }[] = [
	{ place: "sort", title: "To sort" },
	{ place: "picked", title: "Picked up" },
	{ place: "waiting", title: "Waiting on others" },
	{ place: "later", title: "For later" },
];

export type Topic = {
	/** The topic's id: what its items share. */
	key: string;
	/** Whose inbox it is read from. */
	owner: string;
	/** Its items in the owner's view, oldest first. */
	items: InboxItem[];
	/** Its first item, whose title is the topic's. */
	root: InboxItem;
	title: string;
	/** Everyone in it but the owner, in the order they first appear. */
	others: string[];
	place: TopicPlace;
	/** How many items to the owner it has not read: new or noticed, never seen. */
	unread: number;
	/** When anything in it last moved. */
	moved: number;
	/** The items one send of the owner's made for several, when its first item is that send. */
	sends: InboxItem[];
	/** An open item to the owner whose sender has stopped for it. */
	waiting: InboxItem | undefined;
	/** Something in it waits for the person's approval. */
	held: boolean;
	/** A work thread one of its items started is running. */
	live: boolean;
};

/** The topic an item is in; one kept before topics were is its own. */
export const topicOf = (item: InboxItem): string => item.topic ?? item.id;

const UNSORTED = new Set(["new", "noticed", "seen"]);
const isOpen = (item: InboxItem) => item.state !== "closed";

/** A teammate's topics out of its view, the one most recently moved first. `working` holds the work threads running now, by key. */
export function topicsOf(view: InboxView, working: ReadonlySet<string> = new Set()): Topic[] {
	const byTopic = new Map<string, InboxItem[]>();
	for (const item of itemsOf(view).values()) {
		const key = topicOf(item);
		byTopic.set(key, [...(byTopic.get(key) ?? []), item]);
	}
	return [...byTopic.entries()].map(([key, items]) => topic(view.personaId, key, items, working)).sort((a, b) => b.moved - a.moved);
}

/** One topic of `owner`'s, out of its items. */
export function topic(owner: string, key: string, items: readonly InboxItem[], working: ReadonlySet<string> = new Set()): Topic {
	// Oldest first; of two sent the same moment, the one that answers nothing first.
	const sorted = [...items].sort((a, b) => a.ts - b.ts || Number(a.replyTo !== undefined) - Number(b.replyTo !== undefined));
	const root = sorted[0]!;
	const toMe = sorted.filter((item) => item.to === owner && item.held !== true);
	const fromMe = sorted.filter((item) => item.from === owner);
	const sends = root.sendId === undefined || root.from !== owner ? [] : sorted.filter((item) => item.sendId === root.sendId);
	let place: TopicPlace = "closed";
	if (toMe.some((item) => UNSORTED.has(item.state))) place = "sort";
	else if (toMe.some((item) => item.state === "taken")) place = "picked";
	else if (fromMe.some(isOpen)) place = "waiting";
	else if (toMe.some((item) => item.state === "later")) place = "later";
	const others: string[] = [];
	for (const item of sorted) for (const one of [item.from, item.to]) if (one !== owner && !others.includes(one)) others.push(one);
	return {
		key,
		owner,
		items: sorted,
		root,
		title: root.title,
		others,
		place,
		unread: toMe.filter((item) => item.state === "new" || item.state === "noticed").length,
		moved: Math.max(...sorted.map(movedAt)),
		sends: sends.length > 1 ? sends : [],
		waiting: toMe.find((item) => isOpen(item) && item.waiting === true),
		held: sorted.some((item) => item.held === true && isOpen(item)),
		live: sorted.some((item) => item.work !== undefined && working.has(item.work.key)),
	};
}

/**
 * Where a topic stands, in a word or a few, for its head: a send to several
 * answers first; one waiting for the person's approval says so; else the
 * newest open item to the owner, else the newest it sent still open, else
 * how its first item ended.
 */
export function topicWord(one: Topic): string {
	if (one.sends.length > 0) return groupWords(one.sends);
	if (one.items.some((item) => item.from === one.owner && item.held === true && isOpen(item))) return "Awaiting approval";
	const openToMe = one.items.filter((item) => item.to === one.owner && item.held !== true && isOpen(item));
	if (openToMe.length > 0) return stateWord(openToMe.at(-1)!);
	const openFromMe = one.items.filter((item) => item.from === one.owner && isOpen(item));
	if (openFromMe.length > 0) return stateWord(openFromMe.at(-1)!);
	return stateWord(one.root);
}

/** A topic's mark in the list, as an item's was: unread a ring, read a half; picked up, waiting on others, for later and closed as their own. */
export function topicMark(one: Topic): string {
	switch (one.place) {
		case "sort":
			return one.unread > 0 ? "○" : "◐";
		case "picked":
			return one.items.some((item) => item.state === "taken" && item.work !== undefined) ? "≡" : "→";
		case "waiting":
			return "↗";
		case "later":
			return "◷";
		case "closed":
			return one.items.some((item) => item.outcome === "dropped") ? "✕" : one.items.some((item) => item.outcome === "expired") ? "◌" : "✓";
	}
}

/**
 * A topic's line in the list: where a send to several stands, answers
 * first; whom the owner waits on, and how it stands with them; a for-later's
 * reason; how a topic of one item ended; else who said what last.
 */
export function topicLine(one: Topic, nameOf: (personaId: string) => string | undefined): { text: string; warn: boolean } {
	const name = (id: string) => nameOf(id) ?? "A teammate";
	const last = one.items.at(-1)!;
	if (one.sends.length > 0) return { text: groupWords(one.sends), warn: false };
	if (one.place === "waiting" && last.from === one.owner) return { text: `${name(last.to)} · ${stateWord(last)}`, warn: last.held === true };
	if (one.place === "later") {
		const later = one.items.findLast((item) => item.to === one.owner && item.state === "later");
		return { text: [senderWords(later ?? one.root, nameOf), later?.reason].filter(Boolean).join(" · "), warn: false };
	}
	if (one.place === "closed" && one.items.length === 1) return { text: [senderWords(one.root, nameOf), stateWord(one.root), one.root.reason].filter(Boolean).join(" · "), warn: false };
	return { text: `${senderWords(last, nameOf)}: ${firstLine(bubbleText(last))}`, warn: false };
}

const firstLine = (text: string) => text.split("\n").find((line) => line.trim() !== "")?.trim() ?? "";

/** The most a message says and is still a bubble. */
const BUBBLE_CHARS = 160;

/**
 * Whether a message reads as a bubble: it has no words beyond its title, or
 * its words are one plain paragraph of at most 160 characters with no list,
 * heading, quote, code or file. Anything else is a letter.
 */
export function isShort(item: Pick<InboxItem, "body" | "files">): boolean {
	const body = item.body.trim();
	if (body === "") return true;
	if (body.length > BUBBLE_CHARS || body.includes("\n")) return false;
	if (/^(#{1,6}|[-*+>]|\d+[.)])\s/.test(body) || body.includes("`")) return false;
	return (item.files ?? []).length === 0;
}

/** What a bubble says: its words, or its title when the title is the whole message. */
export const bubbleText = (item: Pick<InboxItem, "title" | "body">): string => (item.body.trim() === "" ? item.title : item.body.trim());

/** The recipients of a send to several, answers first, each with the items of its own lane: the replies that chain back to its item. */
export function lanes(one: Topic): { send: InboxItem; items: InboxItem[] }[] {
	const byId = new Map(one.items.map((item) => [item.id, item]));
	const laneOf = (item: InboxItem): InboxItem | undefined => {
		let at: InboxItem | undefined = item;
		for (let hops = 0; at !== undefined && hops < one.items.length; hops += 1) {
			if (one.sends.includes(at)) return at;
			at = at.replyTo === undefined ? undefined : byId.get(at.replyTo);
		}
		return undefined;
	};
	const rank = (item: InboxItem) => ANSWERS_FIRST.indexOf(stateWord(item));
	return [...one.sends]
		.sort((a, b) => rank(a) - rank(b))
		.map((send) => ({ send, items: one.items.filter((item) => item !== send && laneOf(item) === send) }));
}

/** Whether every word of `query` is in the topic: its titles, its words, and who is in it. */
export function topicMatches(one: Topic, query: string, nameOf: (personaId: string) => string | undefined): boolean {
	const words = query.toLowerCase().split(/\s+/).filter(Boolean);
	const hay = one.items.flatMap((item) => [item.title, item.body, nameOf(item.from) ?? "", nameOf(item.to) ?? ""]).join(" ").toLowerCase();
	return words.every((word) => hay.includes(word));
}

/** What the teammate has not read, newest first: the composer's tray. */
export const unreadItems = (view: InboxView | undefined): InboxItem[] => (view?.open ?? []).filter((item) => item.state === "new" || item.state === "noticed");

/** The items a newer Hotline saved that belong on a tab: held for the teammate or sent by it, open or closed. */
export function newerIn(view: InboxView, closed: boolean): InboxNewerItem[] {
	return (view.newer ?? []).filter((item) => (item.to === view.personaId || item.from === view.personaId) && (item.state === "closed") === closed);
}
