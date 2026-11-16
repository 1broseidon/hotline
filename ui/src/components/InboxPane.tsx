import { type ReactNode, useEffect, useState } from "react";
import type { InboxItem, InboxView, ThreadId } from "../generated/contract";
import { asideTime } from "../asides";
import type { Person } from "../avatars";
import { ArrowLeftIcon, ChevronRightIcon } from "../icons";
import { alsoSentWords, type InboxSegment, itemsOf, listHeading, movedAt, openGroups, outcomeWord, receiptRows, type Send, sendState, sends, senderWords, stateWord, toWords, useInbox, useInboxItem, wantsWords } from "../inbox";
import { onTablistKey } from "../ui/Menu";
import { Band } from "../ui/Band";
import { PaneClose } from "../ui/PaneClose";
import { Scroll } from "../ui/Scroll";
import { BandKind, PaneHead, TaskCard } from "./thread/Pane";

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
function markOf(item: InboxItem, sent: boolean): string {
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
 * row opens the item in the pane, with Back here; on Sent, one send to
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
	const row = (item: InboxItem, sent = false) => (
		<ItemRow key={item.id} item={item} sent={sent} now={now} nameOf={nameOf} live={item.work !== undefined && working.has(item.work.key)} onOpen={() => onOpenItem(item.id)} />
	);
	let body: ReactNode = null;
	if (view !== undefined && segment === "open") {
		const groups = openGroups(view);
		body =
			view.open.length === 0 ? (
				<p className="px-2 py-3 text-sm text-ink-3">Nothing open.</p>
			) : (
				<>
					<Group title="Unread" items={groups.unread} />
					{groups.unread.map((item) => row(item))}
					<Group title="Picked up" items={groups.taken} />
					{groups.taken.map((item) => row(item))}
					<Group title="For later" items={groups.later} />
					{groups.later.map((item) => row(item))}
				</>
			);
	} else if (view !== undefined && segment === "closed") {
		body = view.closed.length === 0 ? <p className="px-2 py-3 text-sm text-ink-3">Nothing closed.</p> : view.closed.map((item) => row(item));
	} else if (view !== undefined) {
		body =
			view.sent.length === 0 ? (
				<p className="px-2 py-3 text-sm text-ink-3">Nothing sent.</p>
			) : (
				sends(view.sent).map((send) =>
					send.items.length === 1 ? (
						row(send.items[0]!, true)
					) : (
						<SendRows key={send.items[0]!.id} send={send} now={now} nameOf={nameOf} open={unfolded.has(send.items[0]!.id)} onToggle={() => unfold(send.items[0]!.id)} onOpenItem={onOpenItem} />
					),
				)
			);
	}
	return (
		<section className="dock-page" aria-label={`${name}'s inbox`}>
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Inbox</h2>
				<PaneClose onClose={onClose} />
			</Band>
			{view !== undefined && <p className="dock-note instrument">{listHeading(view, segment)}</p>}
			<div className="list-span">
				<div className="segmented" role="tablist" aria-label="Show" onKeyDown={onTablistKey}>
					{SEGMENTS.map((one) => (
						<button key={one.segment} type="button" role="tab" className="segment" aria-selected={segment === one.segment} onClick={() => onSegment(one.segment)}>
							{one.word}
						</button>
					))}
				</div>
			</div>
			<Scroll>
				<div className="px-2 pb-3">{body}</div>
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
	return (
		<h3 className="list-group">
			<span>{title}</span>
			<span>{items.length}</span>
		</h3>
	);
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
		<button type="button" className="list-row" data-inbox={item.id} data-state={sent ? undefined : item.state} aria-label={`${item.title}, ${line}, ${when}`} onClick={onOpen}>
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

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const dated = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });

/** When a receipt happened: its time today, and its date as well before today, so days apart never read as one minute. */
function receiptTime(at: number, now: number = Date.now()): string {
	const today = new Date(now);
	today.setHours(0, 0, 0, 0);
	return at >= today.getTime() ? clock.format(new Date(at)) : dated.format(new Date(at));
}

/** The band's word over an item: where it is read, with Back to the list. */
const INBOX_KIND = { glyph: "", word: "Inbox", family: "", watching: true };

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
	const tone = item?.state === "closed" ? (item.outcome === "dropped" ? "failed" : "done") : "quiet";
	return (
		<section className="dock-page" aria-label={item === undefined ? "Message" : `Message: ${item.title}`}>
			<Band>
				<button type="button" className="control btn-icon -ml-1" title="Back" aria-label="Back" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				<BandKind kind={INBOX_KIND} live={false} tone={tone} state={item === undefined ? "loading" : stateWord(item)} />
				<PaneClose onClose={onClose} />
			</Band>
			{item === undefined ? (
				error !== null && <p className="dock-note selectable" style={{ color: "var(--warn)" }}>{error}</p>
			) : (
				<Scroll>
					<PaneHead
						kind={INBOX_KIND}
						title={item.title}
						state={stateWord(item)}
						meta={[`${senderWords(item, nameOf)} to ${nameOf(item.to)}`, alsoSentWords(item, (id) => people.get(id)?.name), wantsWords(item.wants), item.waiting === true && `${nameOf(item.from)} is waiting`]}
					/>
					{item.held === true && <p className="dock-note">Waiting for you to allow it.</p>}
					{item.body.trim() !== "" && <TaskCard eyebrow="Message" text={item.body} />}
					<h4 className="pane-section eyebrow">Receipts</h4>
					<ol className="inbox-receipts">
						{receiptRows(item, titleOf).map((row) => (
							<li key={`${row.at}:${row.word}`}>
								<span className="inbox-receipt-at">{receiptTime(row.at)}</span>
								<span className="inbox-receipt-word">{row.word}</span>
								{row.detail !== undefined && <span className="inbox-receipt-detail selectable">{row.detail}</span>}
							</li>
						))}
					</ol>
					{item.work !== undefined && (
						<>
							<h4 className="pane-section eyebrow">Work thread</h4>
							<div className="px-2">
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
							<h4 className="pane-section eyebrow">In reply to</h4>
							<div className="px-2">
								<LinkedItem title={titleOf(item.replyTo) ?? "Original message"} line={`${nameOf(item.to)} to ${nameOf(item.from)}`} onOpen={() => onOpenItem(item.replyTo!)} />
							</div>
						</>
					)}
					{detail!.replies.length > 0 && (
						<>
							<h4 className="pane-section eyebrow">Replies</h4>
							<div className="px-2 pb-3">
								{detail!.replies.map((reply) => (
									<LinkedItem key={reply.id} title={reply.title} line={`${senderWords(reply, nameOf)} · ${asideTime(reply.ts)}`} onOpen={() => onOpenItem(reply.id)} />
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

/** The clock the rows' times read, ticking every `every` ms while the pane is open. */
function useNow(every: number): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const tick = window.setInterval(() => setNow(Date.now()), every);
		return () => window.clearInterval(tick);
	}, [every]);
	return now;
}
