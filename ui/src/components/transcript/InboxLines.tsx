import { createContext, useContext } from "react";
import type { InboxItem } from "../../generated/contract";
import type { Person } from "../../avatars";
import { ChevronRightIcon, InboxIcon } from "../../icons";
import { type InboxSent, type InboxSorted, readWords, sentState, sortedWord, toWords } from "../../inbox";
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
	work: ReadonlySet<string>;
	onOpen?: ((itemId: string) => void) | undefined;
};

export const InboxRows = createContext<InboxRowsContext>({ items: new Map(), work: new Set() });

/**
 * Items a teammate sent, as one quiet row in the conversation they were sent
 * from: the tray, the title, whom it went to, and where it stands now, read
 * live from the sender's inbox ("To Juno · Taken"; "To Juno, Ivy, Huck · 2
 * of 3 seen"). A press opens the item in the pane; several sent together
 * open the first.
 */
export function InboxSentRow({ title, sent, people }: { title: string; sent: InboxSent[]; people: ReadonlyMap<string, Person> | undefined }) {
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
			<p className="link-row" data-kind="inbox">
				{words}
			</p>
		);
	}
	return (
		<button type="button" className="link-row" data-kind="inbox" aria-label={`${label}. Open it`} onClick={() => onOpen(sent[0]!.item)}>
			{words}
			<span className="link-row-open">Open ›</span>
		</button>
	);
}

/**
 * The receiver's sort of what it read, on its main conversation: one quiet
 * line among the turn's, "Read 3 messages", drawn as a path line is
 * (Trail.tsx's FoldLine). Pressed, it unfolds to a row per item in the order
 * given: its title, its sender and a later's reason in the muted ink, and
 * what became of it at the right ("Answered", "Later", "Noted"). A press on
 * a row opens the item in the pane.
 */
export function InboxSortedLine({ event, people }: { event: InboxSorted; people: ReadonlyMap<string, Person> | undefined }) {
	const { onOpen } = useContext(InboxRows);
	const nameOf = (id: string) => people?.get(id)?.name ?? "A teammate";
	return (
		<ol className="trail my-2" data-kind="inbox-sorted">
			<FoldLine id={event.id} said={readWords(event.entries.length)}>
				{event.entries.map((entry) => {
					const muted = [nameOf(entry.from), entry.reason].filter((part): part is string => part !== undefined && part !== "").join(" · ");
					const word = sortedWord(entry);
					return (
						<button
							key={entry.item}
							type="button"
							className="trail-row"
							data-inbox={entry.item}
							aria-label={`${entry.title}, ${muted}, ${word}${onOpen !== undefined ? ". Open it" : ""}`}
							disabled={onOpen === undefined}
							onClick={() => onOpen?.(entry.item)}
						>
							<span className="trail-page">
								{entry.title}
								<span className="text-ink-3"> · {muted}</span>
							</span>
							<span className="trail-meta">{word}</span>
							{onOpen !== undefined && <ChevronRightIcon className="shrink-0 text-ink-3" />}
						</button>
					);
				})}
			</FoldLine>
		</ol>
	);
}
