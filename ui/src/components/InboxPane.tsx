import { Fragment, type ReactNode, useEffect, useState } from "react";
import type { InboxItem, InboxNewerItem, InboxView, ThreadId } from "../generated/contract";
import { asideTime } from "../asides";
import type { Person } from "../avatars";
import { ArrowLeftIcon, ChevronRightIcon } from "../icons";
import { alsoSentWords, type InboxSegment, type InboxSent, itemsOf, listHeading, movedAt, newerIn, openGroups, outcomeWord, receiptRows, type Send, sendState, sends, senderWords, sortedFacts, stateWord, toWords, type TurnInboxRow, useInbox, useInboxItem, wantsWords } from "../inbox";
import { onTablistKey } from "../ui/Menu";
import { Band } from "../ui/Band";
import { PaneClose } from "../ui/PaneClose";
import { Scroll } from "../ui/Scroll";
import { BandKind, PaneHead, TaskCard } from "./thread/Pane";
import { Clamped, Mark, type MarkKind, SectionRule, SpineHead, StateLine } from "./thread/Spine";
import { Markdown } from "./Markdown";

/*
 * A teammate's inbox in the right-hand pane (docs/design/agent-inbox.md, "On
 * the desk"), and one item of it. The person reads here; the teammates act.
 * There is no field and no key that sends, sorts or answers: what moves an
 * item is the teammate's own `update_inbox`.
 */

const SEGMENTS: { segment: InboxSegment; word: string }[] = [
	{ segment: "open", word: "Open" },
	{ segment: "closed", word: "Closed" },
	{ segment: "sent", word: "Sent" },
];

/**
 * A row's mark: unread a ring, read a half; taken, later and closed as their
 * own. A finished one is ticked; one that expired was never picked up, so
 * it keeps an empty ring, dotted.
 */
function markOf(item: Pick<InboxItem, "state" | "outcome" | "work">, sent: boolean): string {
	if (sent) return "↗";
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
 * One teammate's inbox: Open (unread, picked up, for later), Closed and Sent,
 * each newest first, under a line that says what the list on show holds. A
 * row opens the item in the pane, with Back here, and an item taken into a
 * work thread has a row under it that opens the thread; on Sent, one send to
 * several teammates is one row that opens to a row for each.
 */
export function InboxList({
	name,
	view,
	segment,
	people,
	working,
	onSegment,
	onOpenItem,
	onOpenThread,
	onClose,
}: {
	/** Whose inbox: the teammate open beside it. */
	name: string;
	view: InboxView | undefined;
	segment: InboxSegment;
	people: ReadonlyMap<string, Person>;
	/** The teammate's work threads with a turn running, by key. */
	working: ReadonlySet<string>;
	onSegment(segment: InboxSegment): void;
	onOpenItem(itemId: string): void;
	/** Opens the work thread taking an item started, which is its receiver's. */
	onOpenThread(owner: string, thread: ThreadId, title: string): void;
	onClose(): void;
}) {
	// Times of day and days, never a running count: a minute is fine enough.
	const now = useNow(60_000);
	const [unfolded, setUnfolded] = useState<ReadonlySet<string>>(() => new Set());
	const unfold = (id: string) =>
		setUnfolded((was) => {
			const next = new Set(was);
			if (!next.delete(id)) next.add(id);
			return next;
		});
	const nameOf = (id: string) => people.get(id)?.name;
	const row = (item: InboxItem, sent = false) => {
		const live = item.work !== undefined && working.has(item.work.key);
		return (
			<Fragment key={item.id}>
				<ItemRow item={item} sent={sent} now={now} nameOf={nameOf} live={live} onOpen={() => onOpenItem(item.id)} />
				{item.work !== undefined && <WorkRow item={item} live={live} onOpen={() => onOpenThread(item.to, item.work!, item.title)} />}
			</Fragment>
		);
	};
	// What a newer Hotline saved that this build cannot read: listed where it belongs, never opened or moved.
	const newer = view === undefined ? [] : newerIn(view, segment);
	const newerRows = newer.map((item) => <NewerRow key={item.id} item={item} sent={segment === "sent"} now={now} nameOf={nameOf} />);
	let body: ReactNode = null;
	if (view !== undefined && segment === "open") {
		const groups = openGroups(view);
		body =
			view.open.length === 0 && newer.length === 0 ? (
				<p className="spine-empty">Nothing open.</p>
			) : (
				<>
					<Group title="Unread" items={groups.unread} />
					{groups.unread.map((item) => row(item))}
					<Group title="Picked up" items={groups.taken} />
					{groups.taken.map((item) => row(item))}
					<Group title="For later" items={groups.later} />
					{groups.later.map((item) => row(item))}
					{/* Its own section, so it is not read as one of the group above it. */}
					{newer.length > 0 && <SectionRule level={3} className="list-group" title="From a newer version" count={newer.length} />}
					{newerRows}
				</>
			);
	} else if (view !== undefined && segment === "closed") {
		body =
			view.closed.length === 0 && newer.length === 0 ? (
				<p className="spine-empty">Nothing closed.</p>
			) : (
				<>
					{view.closed.map((item) => row(item))}
					{newerRows}
				</>
			);
	} else if (view !== undefined) {
		body =
			view.sent.length === 0 && newer.length === 0 ? (
				<p className="spine-empty">Nothing sent.</p>
			) : (
				<>
					{sends(view.sent).map((send) =>
						send.items.length === 1 ? (
							row(send.items[0]!, true)
						) : (
							<SendRows key={send.items[0]!.id} send={send} now={now} nameOf={nameOf} open={unfolded.has(send.items[0]!.id)} onToggle={() => unfold(send.items[0]!.id)} onOpenItem={onOpenItem} />
						),
					)}
					{newerRows}
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
				{view !== undefined && <p className="spine-count instrument">{listHeading(view, segment)}</p>}
				<div className="list-span">
					<div className="segmented" role="tablist" aria-label="Show" onKeyDown={onTablistKey}>
						{SEGMENTS.map((one) => (
							<button key={one.segment} type="button" role="tab" className="segment" aria-selected={segment === one.segment} onClick={() => onSegment(one.segment)}>
								{one.word}
							</button>
						))}
					</div>
				</div>
			</SpineHead>
			<Scroll>
				<div className="spine-body">{body}</div>
			</Scroll>
		</section>
	);
}

/**
 * One send to several teammates on Sent: one row, its title over whom it
 * reached and where they stand together ("To Huck, Quill, Timo · 2 of 3
 * seen"), that opens to a row for each teammate, each opening its item.
 */
function SendRows({
	send,
	now,
	nameOf,
	open,
	onToggle,
	onOpenItem,
}: {
	send: Send;
	now: number;
	nameOf(personaId: string): string | undefined;
	open: boolean;
	onToggle(): void;
	onOpenItem(itemId: string): void;
}) {
	const first = send.items[0]!;
	const line = `To ${toWords(send.items.map((item) => item.to), nameOf)} · ${sendState(send)}`;
	const when = asideTime(first.ts, now);
	return (
		<>
			<button type="button" className="list-row" data-send={first.id} aria-expanded={open} aria-label={`${first.title}, ${line}, ${when}`} onClick={onToggle}>
				<span className="list-mark" aria-hidden="true">
					↗
				</span>
				<span className="min-w-0 flex-1">
					<span className="list-title">
						<span className="list-title-text">{first.title}</span>
						<ChevronRightIcon className="list-unfold" />
					</span>
					<span className="list-line">{line}</span>
				</span>
				<span className="list-when">{when}</span>
			</button>
			{open &&
				send.items.map((item) => (
					<button key={item.id} type="button" className="list-row list-row-sub" data-inbox={item.id} aria-label={`To ${toWords([item.to], nameOf)}, ${stateWord(item)}`} onClick={() => onOpenItem(item.id)}>
						<span className="min-w-0 flex-1">
							<span className="list-line">
								To {toWords([item.to], nameOf)} · {stateWord(item)}
							</span>
						</span>
					</button>
				))}
		</>
	);
}

function Group({ title, items }: { title: string; items: InboxItem[] }) {
	if (items.length === 0) return null;
	return <SectionRule level={3} className="list-group" title={title} count={items.length} />;
}

/**
 * An item as a row: its mark, the title, who it is from (or to, for one
 * sent) with what it waits on or why it moved, and a time: when it was
 * sent, or last moved. One whose sender stopped for it says "waiting" in its
 * line and its time as every row does, never a countdown.
 */
function ItemRow({
	item,
	sent,
	now,
	nameOf,
	live,
	onOpen,
}: {
	item: InboxItem;
	sent: boolean;
	now: number;
	nameOf(personaId: string): string | undefined;
	live: boolean;
	onOpen(): void;
}) {
	const unsorted = item.state === "new" || item.state === "noticed" || item.state === "seen";
	const waiting = !sent && unsorted && item.waiting === true;
	const line = sent
		? `To ${toWords([item.to], nameOf)} · ${stateWord(item)}`
		: [
				senderWords(item, nameOf),
				waiting && "waiting",
				item.state === "seen" && "read",
				item.state === "closed" && outcomeWord(item.outcome),
				item.reason,
			]
				.filter((part): part is string => typeof part === "string" && part !== "")
				.join(" · ");
	const when = asideTime(sent ? item.ts : movedAt(item), now);
	return (
		<button type="button" className="list-row" data-inbox={item.id} data-state={sent ? undefined : item.state} data-live={live || undefined} aria-label={`${item.title}, ${line}, ${when}`} onClick={onOpen}>
			<span className="list-mark" aria-hidden="true">
				{markOf(item, sent)}
			</span>
			<span className="min-w-0 flex-1">
				<span className="list-title">
					<span className="list-title-text">{item.title}</span>
				</span>
				<span className="list-line">{line}</span>
			</span>
			<span className="list-when">
				{when}
				{live && <span aria-hidden="true" className="dock-dot beat" data-state="running" />}
			</span>
		</button>
	);
}

/**
 * An item a newer Hotline saved in a shape this build cannot read: a plain
 * row, its title and whom it is from (or to), that says so. Nothing here
 * opens it, and nothing on this desk can change it.
 */
function NewerRow({ item, sent, now, nameOf }: { item: InboxNewerItem; sent: boolean; now: number; nameOf(personaId: string): string | undefined }) {
	const who = sent ? `To ${toWords([item.to], nameOf)}` : (nameOf(item.from) ?? "A teammate");
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

/**
 * Under an item picked up into a work thread, the thread: one row set in
 * under it, as a send's rows are, on the text edge with no mark of its own
 * (the item's ≡ in the gutter is the thread's, in the accent while it runs),
 * that opens the thread with Back to the inbox.
 */
function WorkRow({ item, live, onOpen }: { item: InboxItem; live: boolean; onOpen(): void }) {
	return (
		<button type="button" className="list-row list-row-sub" data-work={item.id} aria-label={`Work thread: ${item.title}. Open it`} onClick={onOpen}>
			<span className="min-w-0 flex-1">
				<span className="list-line">Work thread{live ? " · Running" : ""}</span>
			</span>
		</button>
	);
}

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const dated = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });

/** When a receipt happened: its time today, and its date as well before today, so days apart never read as one minute. */
function receiptTime(at: number, now: number = Date.now()): string {
	const today = new Date(now);
	today.setHours(0, 0, 0, 0);
	return at >= today.getTime() ? clock.format(new Date(at)) : dated.format(new Date(at));
}

/** Where an item stands, as the one mark vocabulary says it: open and waiting on someone is a ring; closed is done, dropped failed, expired a dotted ring. */
function itemMark(item: InboxItem): MarkKind {
	if (item.state !== "closed") return "waiting";
	return item.outcome === "dropped" ? "failed" : item.outcome === "expired" ? "expired" : "done";
}

/** The band's word over an item: where it is read, with Back to the list. */
const INBOX_KIND = { glyph: "", word: "Inbox", watching: true };

/**
 * One item in the pane: who sent it to whom and what it wants, its words,
 * its receipts as a timeline, the work thread taking it started, and the
 * items that answer it (or the one it answers), each opening here.
 */
export function InboxItemPane({
	itemId,
	people,
	onBack,
	onClose,
	onOpenItem,
	onOpenThread,
}: {
	itemId: string;
	people: ReadonlyMap<string, Person>;
	onBack(): void;
	onClose(): void;
	onOpenItem(itemId: string): void;
	/** Opens the work thread taking the item started, which is its receiver's. */
	onOpenThread(owner: string, thread: ThreadId, title: string): void;
}) {
	const { detail, error } = useInboxItem(itemId);
	const nameOf = (id: string) => people.get(id)?.name ?? "A teammate";
	const item = detail?.item;
	// What the item names by id: one it was combined into is its receiver's, one it answers is its sender's.
	const held = itemsOf(useInbox(item?.to));
	const answered = itemsOf(useInbox(item?.replyTo !== undefined ? item.from : undefined));
	const titleOf = (id: string) => held.get(id)?.title ?? answered.get(id)?.title;
	const receipts = item === undefined ? [] : receiptRows(item, titleOf);
	// Who and what, in one line: "Goldie to Huck · wants work done".
	const whoWhat = item === undefined ? "" : [`${senderWords(item, nameOf)} to ${nameOf(item.to)}`, alsoSentWords(item, (id) => people.get(id)?.name), wantsWords(item.wants)].filter((part) => part !== undefined && part !== "").join(" · ");
	return (
		<section className="dock-page" aria-label={item === undefined ? "Message" : `Message: ${item.title}`}>
			<Band>
				<button type="button" className="control btn-icon pane-back" title="Back" aria-label="Back" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				<BandKind kind={INBOX_KIND} state={item === undefined ? "loading" : stateWord(item)} />
				<PaneClose onClose={onClose} />
			</Band>
			{item === undefined ? (
				error !== null && <p className="dock-note selectable" style={{ color: "var(--warn)" }}>{error}</p>
			) : (
				<Scroll>
					<PaneHead
						title={item.title}
						line={<StateLine word={stateWord(item)} detail={item.waiting === true && item.state !== "closed" ? `${nameOf(item.from)} is waiting` : undefined} time={receiptTime(movedAt(item))} />}
					/>
					{item.held === true && <p className="dock-note">Waiting for you to allow it.</p>}
					{/* Who and what in one line at the end of the message's rule, or under the state line when there are no words: no facts of their own. */}
					{item.body.trim() !== "" ? (
						<TaskCard eyebrow="Message" text={item.body} aside={<span className="spine-rule-aside">{whoWhat}</span>} />
					) : (
						<p className="spine-who">{whoWhat}</p>
					)}
					<SectionRule title="Receipts" count={receipts.length} />
					<ol className="inbox-receipts spine-list">
						{receipts.map((row, at) => (
							<li key={`${row.at}:${row.word}`}>
								<Mark kind={at === receipts.length - 1 ? itemMark(item) : "done"} />
								<span className="inbox-receipt-word">{row.word}</span>
								{row.detail !== undefined && <span className="inbox-receipt-detail selectable">{row.detail}</span>}
								<span className="inbox-receipt-at">{receiptTime(row.at)}</span>
							</li>
						))}
					</ol>
					{item.work !== undefined && (
						<>
							<SectionRule title="Work thread" />
							<div className="spine-body">
								<button type="button" className="list-row" onClick={() => onOpenThread(item.to, item.work!, item.title)}>
									<span className="list-mark" aria-hidden="true">
										≡
									</span>
									<span className="min-w-0 flex-1">
										<span className="list-title">
											<span className="list-title-text">{item.title}</span>
										</span>
										<span className="list-line">{nameOf(item.to)}</span>
									</span>
								</button>
							</div>
						</>
					)}
					{item.replyTo !== undefined && (
						<>
							<SectionRule title="In reply to" />
							<div className="spine-body">
								<LinkedItem title={titleOf(item.replyTo) ?? "Original message"} line={`${nameOf(item.to)} to ${nameOf(item.from)}`} onOpen={() => onOpenItem(item.replyTo!)} />
							</div>
						</>
					)}
					{detail!.replies.length > 0 && (
						<>
							<SectionRule title="Replies" count={detail!.replies.length} />
							<div className="spine-body">
								{detail!.replies.map((reply) => (
									<div key={reply.id} className="inbox-reply">
										<LinkedItem title={reply.title} line={`${senderWords(reply, nameOf)} · ${asideTime(reply.ts)}`} onOpen={() => onOpenItem(reply.id)} />
										{reply.body.trim() !== "" && (
											<Clamped className="spine-said spine-said-quiet" watch={reply.body}>
												<Markdown text={reply.body} />
											</Clamped>
										)}
									</div>
								))}
							</div>
						</>
					)}
				</Scroll>
			)}
		</section>
	);
}

function LinkedItem({ title, line, onOpen }: { title: string; line: string; onOpen(): void }) {
	return (
		<button type="button" className="list-row" onClick={onOpen}>
			<span className="list-mark" aria-hidden="true">
				↩
			</span>
			<span className="min-w-0 flex-1">
				<span className="list-title">
					<span className="list-title-text">{title}</span>
				</span>
				<span className="list-line">{line}</span>
			</span>
		</button>
	);
}

/**
 * What a turn sorted and sent, in the order it did, as its path lists them
 * under Inbox (`turnInbox`): an item it sorted as the sort left it ("From
 * Timo · Answered"), and one it sent with whom it went to and where it
 * stands now, read live from the teammate's inbox. A quiet turn that posted
 * has them here and no line in the chat. A press opens the item, with Back
 * to the path.
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
						? [row.entry.item, row.entry.title, markOf(row.entry, false), sortedFacts(row.entry, nameOf)]
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
