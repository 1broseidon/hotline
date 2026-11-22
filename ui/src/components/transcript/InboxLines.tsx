import { createContext, type ReactNode, useContext } from "react";
import type { InboxItem, InboxSortedEntry } from "../../generated/contract";
import type { Person } from "../../avatars";
import { InboxIcon } from "../../icons";
import { type InboxSent, senderWords, sentState, sortedFacts, topicOf, toWords } from "../../inbox";

/**
 * What the inbox rows of a conversation read: every item the teammate's own
 * inbox view knows, by id (what it sent, what it holds), live from the
 * subscription, and what a press on a row opens. The rows are memoized
 * against their lines; this is how they hear an item move.
 */
export type InboxRowsContext = {
	items: ReadonlyMap<string, InboxItem>;
	/** The work threads taking one of those items started, by key: drawn as work threads, not handoffs. */
	work: ReadonlyMap<string, string>;
	onOpen?: ((itemId: string) => void) | undefined;
};

export const InboxRows = createContext<InboxRowsContext>({ items: new Map(), work: new Map() });

/**
 * Items a teammate sent, as one quiet row in the conversation they were sent
 * from: the tray, the title, whom it went to, and where it stands now, read
 * live from the sender's inbox ("To Juno · Picked up"; "To Juno, Ivy, Huck
 * · 1 done · 1 seen · 1 awaiting approval", answers first). A press opens
 * the topic it is in, which for several sent together is the whole send,
 * every teammate's lane. `quiet`, as a row of a turn's path, it is in plain
 * weight with the tray in grey.
 */
export function InboxSentRow({ title, sent, people, quiet = false }: { title: string; sent: InboxSent[]; people: ReadonlyMap<string, Person> | undefined; quiet?: boolean }) {
	const { items, onOpen } = useContext(InboxRows);
	const to = `To ${toWords(sent.map((one) => one.to), (id) => people?.get(id)?.name)}`;
	const state = sentState({ title, sent }, items);
	const words = (
		<>
			<InboxIcon className="inbox-row-mark" />
			<span className="link-row-words">
				<span className="link-row-title">{title}</span>
				<span className="link-row-outcome">
					· {to}
					{state !== undefined && ` · ${state}`}
				</span>
			</span>
		</>
	);
	const label = `Message: ${title}, ${to}${state !== undefined ? `, ${state}` : ""}`;
	if (onOpen === undefined) {
		return (
			<p className="link-row" data-kind="inbox" data-quiet={quiet || undefined}>
				{words}
			</p>
		);
	}
	return (
		<button type="button" className="link-row" data-kind="inbox" data-quiet={quiet || undefined} aria-label={`${label}. Open it`} onClick={() => onOpen(sent[0]!.item)}>
			{words}
			<span className="link-row-open">Open ›</span>
		</button>
	);
}

/**
 * Over the person's own line that follows up on an inbox topic (their Ask a
 * follow-up, `aboutItem`), the quote it was written under: whom the topic
 * is with, and its title. A press opens the topic. Nothing until the
 * teammate's inbox knows the item.
 */
export function InboxAboutQuote({ itemId, owner, people }: { itemId: string; owner: string; people: ReadonlyMap<string, Person> | undefined }) {
	const { items, onOpen } = useContext(InboxRows);
	const item = items.get(itemId);
	if (item === undefined) return null;
	const first = [...items.values()].filter((one) => topicOf(one) === topicOf(item)).sort((a, b) => a.ts - b.ts)[0] ?? item;
	const other = item.from === owner ? item.to : item.from;
	const who = people?.get(other)?.name ?? "A teammate";
	const said = first.title.replace(/^Re: /, "");
	return (
		<button type="button" className="quote" title="Open the messages" disabled={onOpen === undefined} onClick={() => onOpen?.(itemId)}>
			<span className="quote-who">{who}</span>
			<span className="quote-said">{said}</span>
		</button>
	);
}

/**
 * Over a bubble a teammate posted from a turn one inbox item woke: one quiet
 * line saying whose message prompted it and its title ("From Goldie's
 * message · Rerun the Linux build"), Hotline's own notice as Hotline's. A
 * press opens the item. Nothing until the teammate's inbox knows the item.
 */
export function InboxFromRow({ itemId, people }: { itemId: string; people: ReadonlyMap<string, Person> | undefined }) {
	const { items, onOpen } = useContext(InboxRows);
	const item = items.get(itemId);
	if (item === undefined) return null;
	const from = item.notice !== undefined ? "From Hotline's notice" : `From ${senderWords(item, (id) => people?.get(id)?.name)}'s message`;
	const words = (
		<>
			<InboxIcon className="inbox-row-mark" />
			<span className="link-row-words">
				<span className="link-row-outcome">{from} ·</span>
				<span className="link-row-title">{item.title}</span>
			</span>
		</>
	);
	if (onOpen === undefined) {
		return (
			<p className="link-row" data-kind="inbox-from">
				{words}
			</p>
		);
	}
	return (
		<button type="button" className="link-row" data-kind="inbox-from" aria-label={`${from}: ${item.title}. Open it`} onClick={() => onOpen(itemId)}>
			{words}
			<span className="link-row-open">Open ›</span>
		</button>
	);
}

/**
 * A card, a file or a job a teammate made in a quiet turn one inbox item
 * woke (`fromItem`), with the line over it saying whose message prompted
 * it: a colleague is behind it, not the person. Nothing over it otherwise.
 */
export function Prompted({ itemId, people, children }: { itemId: string | undefined; people: ReadonlyMap<string, Person> | undefined; children: ReactNode }) {
	if (itemId === undefined) return children;
	return (
		<div data-kind="prompted">
			<InboxFromRow itemId={itemId} people={people} />
			{children}
		</div>
	);
}

/**
 * One item a sort moved, as a row of a turn's path: the tray, its title, whom it
 * is from (Hotline for the desk's own notice, whoever it is about), what
 * became of it in the word the sent row and the panes use, and a later's
 * reason. A press opens the item.
 */
export function InboxSortedRow({ entry, people }: { entry: InboxSortedEntry; people: ReadonlyMap<string, Person> | undefined }) {
	const { onOpen } = useContext(InboxRows);
	const facts = sortedFacts(entry, (id) => people?.get(id)?.name);
	const words = (
		<>
			<InboxIcon className="inbox-row-mark" />
			<span className="link-row-words">
				<span className="link-row-title">{entry.title}</span>
				<span className="link-row-outcome">· {facts}</span>
			</span>
		</>
	);
	if (onOpen === undefined) {
		return (
			<p className="link-row" data-kind="inbox-sorted" data-inbox-item={entry.item}>
				{words}
			</p>
		);
	}
	return (
		<button type="button" className="link-row" data-kind="inbox-sorted" data-inbox-item={entry.item} aria-label={`${entry.title}, ${facts}. Open it`} onClick={() => onOpen(entry.item)}>
			{words}
			<span className="link-row-open">Open ›</span>
		</button>
	);
}
