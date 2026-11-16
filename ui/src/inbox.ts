import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import type { InboxItem, InboxItemDetail, InboxOutcome, InboxReceipt, InboxSortedEntry, InboxView, InboxWants, TranscriptEvent } from "./generated/contract";
import { wire } from "./wire";

/*
 * The inbox on the desk (docs/design/agent-inbox.md, "On the desk"): what a
 * teammate sent and where each item stands, how a receiver sorted what it
 * read, and its inbox as the pane lists it. Teammates act on items; the
 * person reads them. Nothing here sends, sorts or answers anything.
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
 * The work threads taking an item started, by key, out of what a view knows:
 * the receiver's own (open and closed) and those its sent items started.
 * The desk opens such a thread with the item's sender as its opener, as a
 * handoff was; this is how the window tells it from a handoff of old.
 */
export function inboxWork(items: ReadonlyMap<string, InboxItem>): ReadonlySet<string> {
	const keys = new Set<string>();
	for (const item of items.values()) if (item.work?.kind === "side") keys.add(item.work.key);
	return keys;
}

/** The same, for a teammate: live from its inbox. */
export function useInboxWork(personaId: string | undefined): ReadonlySet<string> {
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

/** What the sender wants of it, as the pane's facts say it. */
export function wantsWords(wants: InboxWants): string {
	switch (wants) {
		case "answer":
			return "wants an answer";
		case "work":
			return "wants work done";
		case "nothing":
			return "wants nothing back";
	}
}

/** "2 unread", the composer's chip and the menu's count. */
export const unreadWords = (count: number): string => `${count > 9 ? "9+" : String(count)} unread`;

/**
 * Whether the receiver has read the item's words: it was seen, or sorted
 * after being seen. One closed straight from its title, or dropped while
 * held, never was.
 */
export function wasSeen(item: InboxItem): boolean {
	if (item.held) return false;
	if (item.state === "seen" || item.state === "taken" || item.state === "later") return true;
	return item.receipts.some((receipt) => receipt.state === "seen" || receipt.state === "taken" || receipt.state === "later");
}

/* ---------------------------------------------------- the sender's rows */

/** Items sent together: one or more `inbox_sent` lines with the same title, side by side on the tape. */
export type SentGroup = { title: string; sent: InboxSent[] };

/** Recipients by name, three at most and the rest counted: "Juno, Ivy, Huck +2". */
export function toWords(ids: readonly string[], nameOf: (personaId: string) => string | undefined): string {
	const names = ids.map((id) => nameOf(id) ?? "a teammate");
	return names.length <= 3 ? names.join(", ") : `${names.slice(0, 3).join(", ")} +${names.length - 3}`;
}

/** Who else the send that made an item reached, as its pane's head says it: "also sent to Ivy, Huck". */
export function alsoSentWords(item: InboxItem, nameOf: (personaId: string) => string | undefined): string | undefined {
	const others = item.alsoTo ?? [];
	return others.length === 0 ? undefined : `also sent to ${toWords(others, nameOf)}`;
}

/**
 * Where items sent together stand, from the sender's own view: one item's
 * state word; for several, the word they share, else how many of them were
 * seen ("2 of 3 seen"). Nothing while none of them is known yet.
 */
export function sentState(group: SentGroup, items: ReadonlyMap<string, InboxItem>): string | undefined {
	const known = group.sent.flatMap((one) => {
		const item = items.get(one.item);
		return item === undefined ? [] : [item];
	});
	return together(known, group.sent.length);
}

/** Where `count` items sent together stand, from the `known` of them: one's word, the word they share, else how many were seen. */
function together(known: readonly InboxItem[], count: number): string | undefined {
	if (known.length === 0) return undefined;
	if (count === 1) return stateWord(known[0]!);
	const words = known.map(stateWord);
	if (known.length === count && words.every((word) => word === words[0])) return words[0];
	return `${known.filter(wasSeen).length} of ${count} seen`;
}

/** The items one `send` made, in the order it made them: one for each teammate it reached. */
export type Send = { items: InboxItem[] };

/**
 * A sender's list as sends: the items one `send` made for several teammates
 * stand side by side in it, under one title, each naming the others in
 * `alsoTo`, so a run of them that agree on the title and on who the send
 * reached, each to a different teammate, is one send. The list is newest
 * first, so a send's items are put back in the order it made them, which is
 * the order its row in the conversation names them.
 */
export function sends(sent: readonly InboxItem[]): Send[] {
	const reached = (item: InboxItem) => [item.to, ...(item.alsoTo ?? [])].sort().join(" ");
	const out: Send[] = [];
	for (const item of sent) {
		const last = out.at(-1)?.items;
		const first = last?.[0];
		const joins =
			first !== undefined &&
			(first.alsoTo ?? []).length > 0 &&
			first.title === item.title &&
			reached(first) === reached(item) &&
			!last!.some((one) => one.to === item.to);
		if (joins) last!.unshift(item);
		else out.push({ items: [item] });
	}
	return out;
}

/** Where a send stands, as its row on Sent says it: "Seen", or "2 of 3 seen". */
export const sendState = (send: Send): string => together(send.items, send.items.length) ?? "";

/* ----------------------------------------------------- the inbox's line */

/** What became of an entry of a sort, in a word, at the right of its row. */
export function sortedWord(entry: InboxSortedEntry): string {
	if (entry.state === "taken") return "Picked up";
	if (entry.state === "later") return "Later";
	if (entry.state === "closed") return outcomeWord(entry.outcome);
	return entry.state.charAt(0).toUpperCase() + entry.state.slice(1);
}

/** "Inbox · read 3, sent 1": the line for a run of a teammate's inbox activity, saying only what it has. */
export const runWords = (read: number, sent: number): string =>
	`Inbox · ${[read > 0 && `read ${read}`, sent > 0 && `sent ${sent}`].filter((part): part is string => part !== false).join(", ")}`;

/* ------------------------------------------------------------ the pane */

/** The pane's three segments. */
export type InboxSegment = "open" | "closed" | "sent";

/** What is open, as the pane groups it: not yet sorted (new, noticed, seen), taken, for later; each newest first. */
export function openGroups(view: InboxView): { unread: InboxItem[]; taken: InboxItem[]; later: InboxItem[] } {
	return {
		unread: view.open.filter((item) => item.state === "new" || item.state === "noticed" || item.state === "seen"),
		taken: view.open.filter((item) => item.state === "taken"),
		later: view.open.filter((item) => item.state === "later"),
	};
}

/** What the teammate has not read, newest first: the composer's tray. */
export const unreadItems = (view: InboxView | undefined): InboxItem[] => (view?.open ?? []).filter((item) => item.state === "new" || item.state === "noticed");

/** The pane's heading line for the list it shows: the open counts, else how many closed or sent, else that there are none. */
export function listHeading(view: InboxView, segment: InboxSegment): string {
	if (segment === "open") return openCounts(view) || "Nothing open";
	const count = segment === "closed" ? view.closed.length : sends(view.sent).length;
	return count === 0 ? `Nothing ${segment}` : `${count} ${segment}`;
}

/** The open count as the pane heads it: "2 unread · 1 picked up · 1 for later". */
function openCounts(view: InboxView): string {
	const { taken, later } = openGroups(view);
	return [view.unread > 0 && `${view.unread} unread`, taken.length > 0 && `${taken.length} picked up`, later.length > 0 && `${later.length} for later`]
		.filter((part): part is string => part !== false)
		.join(" · ");
}

/** When an item last moved: its newest receipt, or when it was sent. */
export const movedAt = (item: InboxItem): number => item.receipts.at(-1)?.at ?? item.ts;

/** One line of an item's receipts: when, the state's word, and why where it says. */
export type ReceiptRow = { at: number; word: string; detail?: string };

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
	return rows;
}

/** A first receipt this long after the send was the first-contact card's yes, not the send. */
const LET_IN_AFTER = 5_000;
