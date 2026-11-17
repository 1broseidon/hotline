import { createContext, memo, type ReactNode, useContext } from "react";
import type { InboxItem, InboxSortedEntry } from "../../generated/contract";
import type { Person } from "../../avatars";
import type { LinkEvent } from "../../dock";
import { InboxIcon } from "../../icons";
import { type InboxSent, runWords, senderWords, sentState, sortedWord, toWords } from "../../inbox";
import type { Block } from "../../scheduledRuns";
import { LinkRow } from "./LinkRow";
import { Steps } from "./Steps";
import { FoldLine } from "./Trail";

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
 * live from the sender's inbox ("To Juno · Taken"; "To Juno, Ivy, Huck · 2
 * of 3 seen"). A press opens the item in the pane; several sent together
 * open the first. `quiet`, in an inbox run or as what came of a turn the
 * person asked for, it is in plain weight with the tray in grey.
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
 * A run of the teammate's inbox activity, as one quiet line among the
 * conversation's ("Inbox · read 3, sent 1"), drawn as a path line is
 * (Trail.tsx's FoldLine). Pressed, it opens in place to its rows in the
 * order they happened, with no box around them: a row for each item it
 * sorted (its title, whom it is from, what became of it, and a later's
 * reason), each send (InboxSentRow), the work an item started once it is
 * over, and each quiet turn's time, which opens that turn's path as the
 * line under a reply does. Every row is quiet: plain weight, the tray in
 * grey. A press on an item's row opens the item.
 */
export const InboxRun = memo(function InboxRun({
	run,
	owner,
	people,
	onOpenThread,
	onOpenWork,
	workOpen,
}: {
	run: Extract<Block, { kind: "inboxRun" }>;
	/** Whose conversation this is. */
	owner: string;
	people: ReadonlyMap<string, Person> | undefined;
	onOpenThread?: ((link: LinkEvent) => void) | undefined;
	onOpenWork?: ((blockId: string) => void) | undefined;
	workOpen?: string | null | undefined;
}) {
	return (
		<ol className="trail my-2" data-kind="inbox-run">
			<FoldLine id={run.id} said={runWords(run.read, run.sent)} detail="inbox-run-rows">
				{run.members.map((member) => {
					if (member.kind === "inbox") return <InboxSentRow key={member.id} title={member.title} sent={member.sent} people={people} quiet />;
					if (member.kind === "steps")
						return (
							<Steps
								key={member.id}
								id={member.id}
								items={member.items}
								live={false}
								shown={false}
								rowSays={member.rowSays}
								{...(onOpenWork !== undefined ? { onOpenWork, open: workOpen === member.id } : {})}
							/>
						);
					if (member.kind !== "event") return null;
					const event = member.event;
					if (event.kind === "inbox_sorted") return event.entries.map((entry, at) => <InboxSortedRow key={`${event.id}:${at}`} entry={entry} people={people} />);
					if (event.kind === "link") return <LinkRow key={event.id} link={event} owner={owner} people={people} inboxWork onOpen={() => onOpenThread?.(event)} />;
					return null;
				})}
			</FoldLine>
		</ol>
	);
});

/**
 * One item a sort moved, as a row of the run: the tray, its title, whom it
 * is from (Hotline for the desk's own notice, whoever it is about), what
 * became of it in the word the sent row and the panes use, and a later's
 * reason. A press opens the item.
 */
function InboxSortedRow({ entry, people }: { entry: InboxSortedEntry; people: ReadonlyMap<string, Person> | undefined }) {
	const { onOpen } = useContext(InboxRows);
	const from = `From ${entry.notice !== undefined ? "Hotline" : (people?.get(entry.from)?.name ?? "a teammate")}`;
	const facts = [from, sortedWord(entry), entry.reason].filter((part): part is string => part !== undefined && part !== "").join(" · ");
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
