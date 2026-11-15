import { type CSSProperties, createContext, useContext } from "react";
import type { InboxItem } from "../../generated/contract";
import { asideTime } from "../../asides";
import type { Person } from "../../avatars";
import { InboxIcon } from "../../icons";
import { type InboxSent, type InboxSorted, readWords, sentState, sortedCounts, sortedRows, toWords } from "../../inbox";

/**
 * What the inbox rows of a conversation read: every item the teammate's own
 * inbox view knows, by id (what it sent, what it holds), live from the
 * subscription, and what a press on a row opens. The rows are memoized
 * against their lines; this is how they hear an item move.
 */
export type InboxRowsContext = { items: ReadonlyMap<string, InboxItem>; onOpen?: ((itemId: string) => void) | undefined };

export const InboxRows = createContext<InboxRowsContext>({ items: new Map() });

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
 * The receiver's sort of what it read, on its main conversation: one card,
 * "Read 4 messages" with the time and the counts, then each entry in the
 * order given, taken and later ones numbered, one combined into another
 * indented under it. Each says its title, its sender (waiting, when the item
 * says the sender stopped for it), and its state at the right, with the
 * reason a later or a drop gave. A sort of one item is a quiet row instead.
 * A press opens the item in the pane.
 */
export function InboxSortedCard({ event, people }: { event: InboxSorted; people: ReadonlyMap<string, Person> | undefined }) {
	const { items, onOpen } = useContext(InboxRows);
	const nameOf = (id: string) => people?.get(id)?.name ?? "A teammate";
	const rows = sortedRows(event.entries);
	if (rows.length === 1) {
		const { entry, word } = rows[0]!;
		const said = `· From ${nameOf(entry.from)} · ${word}${entry.reason !== undefined && entry.reason !== "" ? `: ${entry.reason}` : ""}`;
		const words = (
			<>
				<InboxIcon className="inbox-row-mark" />
				<span className="link-row-words">
					<span className="link-row-title">{entry.title}</span>
					<span className="link-row-outcome">{said}</span>
				</span>
			</>
		);
		return onOpen === undefined ? (
			<p className="link-row" data-kind="inbox-sorted">
				{words}
			</p>
		) : (
			<button type="button" className="link-row" data-kind="inbox-sorted" aria-label={`Read: ${entry.title} ${said}. Open it`} onClick={() => onOpen(entry.item)}>
				{words}
				<span className="link-row-open">Open ›</span>
			</button>
		);
	}
	const counts = sortedCounts(event.entries);
	return (
		<section className="inbox-sorted" aria-label={`${readWords(rows.length)}, ${counts}`}>
			<header className="inbox-sorted-head">
				<span className="inbox-sorted-title">
					<InboxIcon />
					{readWords(rows.length)}
				</span>
				<span>{asideTime(event.ts)}</span>
				<span className="inbox-sorted-counts">{counts}</span>
			</header>
			<ol>
				{rows.map((row) => {
					const { entry } = row;
					const waiting = items.get(entry.item)?.waiting === true;
					const line = [nameOf(entry.from), waiting && "waiting", entry.reason].filter((part): part is string => typeof part === "string" && part !== "").join(" · ");
					const body = (
						<>
							<span className="list-mark" aria-hidden="true">
								{row.number ?? (row.depth > 0 ? "" : "–")}
							</span>
							<span className="min-w-0 flex-1">
								<span className="list-title">
									<span className="list-title-text">{entry.title}</span>
								</span>
								<span className="list-line">{line}</span>
							</span>
							<span className="list-when" data-taken={row.taken || undefined}>
								{row.word}
							</span>
						</>
					);
					return (
						<li key={entry.item} data-depth={row.depth || undefined} style={row.depth > 0 ? ({ "--depth": row.depth } as CSSProperties) : undefined}>
							{onOpen === undefined ? (
								<div className="list-row">{body}</div>
							) : (
								<button
									type="button"
									className="list-row"
									aria-label={`${row.number !== undefined ? `${row.number}. ` : ""}${entry.title}, ${line}, ${row.word}`}
									onClick={() => onOpen(entry.item)}
								>
									{body}
								</button>
							)}
						</li>
					);
				})}
			</ol>
		</section>
	);
}
