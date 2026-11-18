import { Fragment, type ReactNode, useEffect, useState } from "react";
import type { InboxItem, InboxNewerItem, InboxView } from "../generated/contract";
import { asideTime } from "../asides";
import type { Person } from "../avatars";
import { type InboxSent, itemsOf, newerIn, sortedFacts, stateWord, TOPIC_GROUPS, type Topic, topicLine, topicMark, topicMatches, topicsOf, toWords, type TurnInboxRow, useInbox } from "../inbox";
import { onTablistKey } from "../ui/Menu";
import { Band } from "../ui/Band";
import { PaneClose } from "../ui/PaneClose";
import { Scroll } from "../ui/Scroll";
import { SectionRule, SpineHead } from "./thread/Spine";

/*
 * A teammate's inbox in the right-hand pane (docs/design/agent-inbox.md, "On
 * the desk"): one row per topic, an ask with its answers and follow-ups,
 * which opens as a conversation (InboxTopic.tsx). The person reads here; the
 * teammates act. There is no field and no key that sends, sorts or answers:
 * what moves an item is the teammate's own `update_inbox`, and opening a
 * topic marks nothing seen.
 */

/** The pane's two tabs: what is open on either side, and what is over. */
export type InboxTab = "open" | "closed";

const TABS: { tab: InboxTab; word: string }[] = [
	{ tab: "open", word: "Open" },
	{ tab: "closed", word: "Closed" },
];

/**
 * One teammate's inbox: a search field, then Open (To sort, Picked up,
 * Waiting on others, For later) and Closed, each newest first. A row is a
 * topic, and opens it in the pane with Back here. A search reads every
 * message of every topic, open and closed, and hides the tabs while it
 * shows what matched.
 */
export function InboxList({
	name,
	view,
	tab,
	query,
	people,
	working,
	onTab,
	onQuery,
	onOpenItem,
	onClose,
}: {
	/** Whose inbox: the teammate open beside it. */
	name: string;
	view: InboxView | undefined;
	tab: InboxTab;
	query: string;
	people: ReadonlyMap<string, Person>;
	/** The teammate's work threads with a turn running, by key. */
	working: ReadonlySet<string>;
	onTab(tab: InboxTab): void;
	onQuery(query: string): void;
	/** Opens the topic this item is in, at that item: a row opens its topic at the top. */
	onOpenItem(itemId: string): void;
	onClose(): void;
}) {
	// Times of day and days, never a running count: a minute is fine enough.
	const now = useNow(60_000);
	const nameOf = (id: string) => people.get(id)?.name;
	const topics = view === undefined ? [] : topicsOf(view, working);
	const row = (one: Topic) => <TopicRow key={one.key} topic={one} now={now} nameOf={nameOf} onOpen={() => onOpenItem(one.root.id)} />;
	const newerRows = (closed: boolean) => (view === undefined ? [] : newerIn(view, closed)).map((item) => <NewerRow key={item.id} item={item} owner={view!.personaId} now={now} nameOf={nameOf} />);
	const searching = query.trim() !== "";
	let body: ReactNode = null;
	if (view !== undefined && searching) {
		const found = topics.filter((one) => topicMatches(one, query, nameOf));
		const open = found.filter((one) => one.place !== "closed");
		const closed = found.filter((one) => one.place === "closed");
		body =
			found.length === 0 ? (
				<p className="spine-empty">Nothing matches.</p>
			) : (
				<>
					<Group title="Open" count={open.length} />
					{open.map(row)}
					<Group title="Closed" count={closed.length} />
					{closed.map(row)}
				</>
			);
	} else if (view !== undefined && tab === "open") {
		const newer = newerRows(false);
		const open = topics.filter((one) => one.place !== "closed");
		body =
			open.length === 0 && newer.length === 0 ? (
				<p className="spine-empty">Nothing open.</p>
			) : (
				<>
					{TOPIC_GROUPS.map((group) => {
						const rows = open.filter((one) => one.place === group.place);
						return (
							<Fragment key={group.place}>
								<Group title={group.title} count={rows.length} />
								{rows.map(row)}
							</Fragment>
						);
					})}
					{/* Its own section, so it is not read as one of the group above it. */}
					<Group title="From a newer version" count={newer.length} />
					{newer}
				</>
			);
	} else if (view !== undefined) {
		const newer = newerRows(true);
		const closed = topics.filter((one) => one.place === "closed");
		body =
			closed.length === 0 && newer.length === 0 ? (
				<p className="spine-empty">Nothing closed.</p>
			) : (
				<>
					{closed.map(row)}
					{newer}
				</>
			);
	}
	return (
		<section className="dock-page" aria-label={`${name}'s inbox`}>
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Inbox</h2>
				<PaneClose onClose={onClose} />
			</Band>
			<SpineHead className="spine-head-list">
				<div className="list-search">
					<input
						type="search"
						className="field"
						placeholder="Search messages"
						aria-label="Search messages"
						autoComplete="off"
						spellCheck={false}
						value={query}
						onChange={(change) => onQuery(change.target.value)}
						onKeyDown={(key) => {
							if (key.key === "Escape" && query !== "") {
								key.stopPropagation();
								onQuery("");
							}
						}}
					/>
				</div>
				{!searching && (
					<div className="list-span">
						<div className="segmented" role="tablist" aria-label="Show" onKeyDown={onTablistKey}>
							{TABS.map((one) => (
								<button key={one.tab} type="button" role="tab" className="segment" aria-selected={tab === one.tab} onClick={() => onTab(one.tab)}>
									{one.word}
								</button>
							))}
						</div>
					</div>
				)}
			</SpineHead>
			<Scroll>
				<div className="spine-body">{body}</div>
			</Scroll>
		</section>
	);
}

function Group({ title, count }: { title: string; count: number }) {
	if (count === 0) return null;
	return <SectionRule level={3} className="list-group" title={title} count={count} />;
}

/**
 * A topic as a row: its mark, its title, then who said what last, or where
 * a send stands, and when it last moved. One with something the teammate
 * has not read is in the accent and bold; one waiting for the person's
 * approval says so in amber; a running work thread is the live dot.
 */
function TopicRow({ topic, now, nameOf, onOpen }: { topic: Topic; now: number; nameOf(personaId: string): string | undefined; onOpen(): void }) {
	const line = topicLine(topic, nameOf);
	const when = asideTime(topic.moved, now);
	return (
		<button
			type="button"
			className="list-row"
			data-topic={topic.key}
			data-unread={topic.unread > 0 || undefined}
			data-live={topic.live || undefined}
			aria-label={`${topic.title}, ${line.text}, ${when}`}
			onClick={onOpen}
		>
			<span className="list-mark" aria-hidden="true">
				{topicMark(topic)}
			</span>
			<span className="min-w-0 flex-1">
				<span className="list-title">
					<span className="list-title-text">{topic.title}</span>
				</span>
				<span className="list-line" data-warn={line.warn || undefined}>
					{line.text}
				</span>
			</span>
			<span className="list-when">
				{when}
				{topic.live && <span aria-hidden="true" className="dock-dot beat" data-state="running" />}
			</span>
		</button>
	);
}

/**
 * An item a newer Hotline saved in a shape this build cannot read: a plain
 * row, its title and whom it is from (or to), that says so. Nothing here
 * opens it, and nothing on this desk can change it.
 */
function NewerRow({ item, owner, now, nameOf }: { item: InboxNewerItem; owner: string; now: number; nameOf(personaId: string): string | undefined }) {
	const who = item.from === owner ? `To ${toWords([item.to], nameOf)}` : (nameOf(item.from) ?? "A teammate");
	const line = `${who} · Saved by a newer version of Hotline. It can't be changed here.`;
	const when = asideTime(item.ts, now);
	return (
		<div className="list-row" data-newer={item.id}>
			<span className="list-mark" aria-hidden="true">
				·
			</span>
			<span className="min-w-0 flex-1">
				<span className="list-title">
					<span className="list-title-text">{item.title}</span>
				</span>
				<span className="list-line">{line}</span>
			</span>
			<span className="list-when">{when}</span>
		</div>
	);
}

/** A sorted entry's or an item's mark in a turn's path: unread a ring, read a half; taken, later and closed as their own. */
function markOf(item: Pick<InboxItem, "state" | "outcome" | "work">): string {
	switch (item.state) {
		case "new":
		case "noticed":
			return "○";
		case "seen":
			return "◐";
		case "taken":
			return item.work !== undefined ? "≡" : "→";
		case "later":
			return "◷";
		case "closed":
			return item.outcome === "dropped" ? "✕" : item.outcome === "expired" ? "◌" : "✓";
	}
}

/**
 * What a turn sorted and sent, in the order it did, as its path lists them
 * under Inbox (`turnInbox`): an item it sorted as the sort left it ("From
 * Timo · Answered"), and one it sent with whom it went to and where it
 * stands now, read live from the teammate's inbox. A quiet turn that posted
 * has them here and no line in the chat. A press opens the item's topic,
 * with Back to the path.
 */
export function TurnInbox({
	personaId,
	rows,
	people,
	onOpenItem,
}: {
	personaId: string;
	rows: readonly TurnInboxRow[];
	people: ReadonlyMap<string, Person> | undefined;
	onOpenItem(itemId: string): void;
}) {
	const items = itemsOf(useInbox(personaId));
	const nameOf = (id: string) => people?.get(id)?.name;
	return (
		<section aria-label="Inbox">
			<SectionRule title="Inbox" count={rows.length} />
			{rows.map((row) => {
				const [itemId, title, mark, line] =
					row.kind === "sorted"
						? [row.entry.item, row.entry.title, markOf(row.entry), sortedFacts(row.entry, nameOf)]
						: [row.sent.item, row.sent.title, "↗", sentLine(row.sent, items.get(row.sent.item), nameOf)];
				return (
					<button key={row.key} type="button" className="list-row" data-inbox={itemId} aria-label={`${title}, ${line}. Open it`} onClick={() => onOpenItem(itemId)}>
						<span className="list-mark" aria-hidden="true">
							{mark}
						</span>
						<span className="min-w-0 flex-1">
							<span className="list-title">
								<span className="list-title-text">{title}</span>
							</span>
							<span className="list-line">{line}</span>
						</span>
					</button>
				);
			})}
		</section>
	);
}

/** A send's row in a path: whom it went to, and where it stands once the teammate's inbox knows it. */
function sentLine(sent: InboxSent, item: InboxItem | undefined, nameOf: (personaId: string) => string | undefined): string {
	const to = `To ${toWords([sent.to], nameOf)}`;
	return item === undefined ? to : `${to} · ${stateWord(item)}`;
}

/** The clock the rows' times read, ticking every `every` ms while the pane is open. */
function useNow(every: number): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const tick = window.setInterval(() => setNow(Date.now()), every);
		return () => window.clearInterval(tick);
	}, [every]);
	return now;
}
