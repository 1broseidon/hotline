import { useLayoutEffect, useRef, useState } from "react";
import type { InboxItem, InboxView, ThreadId } from "../generated/contract";
import type { Person } from "../avatars";
import { ArrowLeftIcon, ChevronRightIcon } from "../icons";
import { bubbleText, isShort, lanes, movedAt, receiptRows, stateWord, type Topic, topic, topicOf, topicWord, useInboxItem } from "../inbox";
import { Band } from "../ui/Band";
import { PaneClose } from "../ui/PaneClose";
import { Scroll } from "../ui/Scroll";
import { BandKind, PaneHead } from "./thread/Pane";
import { Clamped, Mark, SectionRule, type Status, StateLine } from "./thread/Spine";
import { Markdown } from "./Markdown";

/*
 * One topic of a teammate's inbox, open in the pane as a conversation
 * (docs/design/agent-inbox.md, "On the desk"): each message in order, the
 * inbox owner's on the right in a raised container and the other side's on
 * the left in the quiet one, never the person's mint. A short message is a
 * bubble and a long one a letter, six lines and Show all (`isShort`). Under
 * each, one word for where it stands, which opens its history in place;
 * Awaiting approval is always written out, in amber, with the way to its
 * card. A send to several is the send, then a lane for each teammate,
 * answers first. Ask a follow-up fills the person's own message box with
 * the topic quoted; nothing here sends, sorts or marks anything seen.
 */

/** What Ask a follow-up hands the person's message box: the item it is about, and the quote it shows. */
export type FollowUp = { aboutItem: string; quote: string };

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const dated = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });

/** A time today, and its date as well before today, so days apart never read as one minute. */
function timeOf(at: number, now: number = Date.now()): string {
	const today = new Date(now);
	today.setHours(0, 0, 0, 0);
	return at >= today.getTime() ? clock.format(new Date(at)) : dated.format(new Date(at));
}

/** The band's word over a topic: where it is read, with Back to the list. */
const INBOX_KIND = { glyph: "", word: "Inbox", watching: true };

/** The topic's one status mark, on its messages' rule: live work, the person's approval, how it ended. */
function statusOf(one: Topic): Status | undefined {
	if (one.live) return "live";
	if (one.held) return "blocked";
	if (one.place !== "closed") return undefined;
	return one.items.some((item) => item.outcome === "dropped") ? "failed" : "stopped";
}

/**
 * The topic the item `itemId` is in, read from its owner's view, scrolled
 * to that item. An item the view no longer holds (older than the newest it
 * is sent) is read on its own, with its replies.
 */
export function InboxTopicPane({
	itemId,
	view,
	people,
	working,
	onBack,
	onClose,
	onOpenThread,
	onFollowUp,
	onGoToCard,
}: {
	itemId: string;
	/** The inbox owner's view, which holds both sides of its topics. */
	view: InboxView | undefined;
	people: ReadonlyMap<string, Person>;
	working: ReadonlySet<string>;
	onBack(): void;
	onClose(): void;
	/** Opens the work thread taking an item started, which is its receiver's. */
	onOpenThread(owner: string, thread: ThreadId, title: string): void;
	/** Fills the person's message box with the topic quoted. */
	onFollowUp(followUp: FollowUp): void;
	/** Opens the card that lets a held item in, in the owner's chat. */
	onGoToCard(to: string): void;
}) {
	const found = view?.open.concat(view.closed, view.sent).find((item) => item.id === itemId);
	const owner = view?.personaId;
	const inView = found === undefined || owner === undefined ? undefined : topic(owner, topicOf(found), [...view!.open, ...view!.closed, ...view!.sent].filter((item) => topicOf(item) === topicOf(found)), working);
	return inView !== undefined ? (
		<TopicPage key={inView.key} topic={inView} focus={itemId} people={people} working={working} onBack={onBack} onClose={onClose} onOpenThread={onOpenThread} onFollowUp={onFollowUp} onGoToCard={onGoToCard} />
	) : (
		<Detached itemId={itemId} owner={owner} people={people} working={working} onBack={onBack} onClose={onClose} onOpenThread={onOpenThread} onFollowUp={onFollowUp} onGoToCard={onGoToCard} />
	);
}

/** An item the owner's view does not hold, read by id with its replies: the topic as far as they go. */
function Detached({ itemId, owner, ...rest }: Omit<Parameters<typeof InboxTopicPane>[0], "view"> & { owner: string | undefined }) {
	const { detail, error } = useInboxItem(itemId);
	if (detail === undefined)
		return (
			<section className="dock-page" aria-label="Message">
				<Band>
					<button type="button" className="control btn-icon pane-back" title="Back" aria-label="Back" onClick={rest.onBack}>
						<ArrowLeftIcon />
					</button>
					<BandKind kind={INBOX_KIND} state="loading" />
					<PaneClose onClose={rest.onClose} />
				</Band>
				{error !== null && <p className="dock-note selectable" style={{ color: "var(--warn)" }}>{error}</p>}
			</section>
		);
	const items = [detail.item, ...detail.replies];
	const whose = owner !== undefined && items.some((item) => item.from === owner || item.to === owner) ? owner : detail.item.to;
	return <TopicPage topic={topic(whose, topicOf(detail.item), items, rest.working)} focus={itemId} {...rest} />;
}

function TopicPage({
	topic: one,
	focus,
	people,
	working,
	onBack,
	onClose,
	onOpenThread,
	onFollowUp,
	onGoToCard,
}: {
	topic: Topic;
	focus: string;
	people: ReadonlyMap<string, Person>;
	working: ReadonlySet<string>;
	onBack(): void;
	onClose(): void;
	onOpenThread(owner: string, thread: ThreadId, title: string): void;
	onFollowUp(followUp: FollowUp): void;
	onGoToCard(to: string): void;
}) {
	const nameOf = (id: string) => people.get(id)?.name ?? "A teammate";
	const [history, setHistory] = useState<ReadonlySet<string>>(() => new Set());
	const toggle = (id: string) =>
		setHistory((was) => {
			const next = new Set(was);
			if (!next.delete(id)) next.add(id);
			return next;
		});
	const page = useRef<HTMLElement>(null);
	// Opened on a message further down (a sent row, a follow-up), the page starts at it.
	useLayoutEffect(() => {
		const at = [...(page.current?.querySelectorAll<HTMLElement>("[data-message]") ?? [])].find((one) => one.dataset["message"] === focus);
		if (at !== undefined && at.offsetTop > 300) at.scrollIntoView?.({ block: "start" });
	}, [focus]);
	const word = topicWord(one);
	const group = one.sends.length > 0;
	const others = one.others.map(nameOf).join(", ");
	const detail = group ? undefined : one.waiting !== undefined ? `${nameOf(one.waiting.from)} is waiting` : `with ${others}`;
	const said = (item: InboxItem, before: string | undefined) => (
		<Message
			key={item.id}
			item={item}
			topic={one}
			nameOf={nameOf}
			named={before !== item.from}
			history={history.has(item.id)}
			live={item.work !== undefined && working.has(item.work.key)}
			onHistory={() => toggle(item.id)}
			onOpenThread={onOpenThread}
			onGoToCard={onGoToCard}
		/>
	);
	const stream = group ? (
		<>
			<div className="inbox-msg" data-side="right" data-message={one.root.id}>
				<div className="inbox-line">
					<p className="inbox-who">{nameOf(one.root.from)}</p>
					{isShort(one.root) ? <div className="inbox-bubble selectable">{bubbleText(one.root)}</div> : <Letter item={one.root} topic={one} nameOf={nameOf} />}
					<p className="inbox-meta">
						<span>{timeOf(one.root.ts)}</span>
						<span>·</span>
						<span>To {one.sends.map((send) => nameOf(send.to)).join(", ")}</span>
					</p>
				</div>
			</div>
			{lanes(one).map(({ send, items }) => {
				const held = send.held === true && send.state !== "closed";
				let before: string | undefined;
				return (
					<div key={send.id} className="inbox-lane" data-message={send.id}>
						<SectionRule
							title={nameOf(send.to)}
							status={held ? "blocked" : send.work !== undefined && working.has(send.work.key) ? "live" : undefined}
							aside={
								<span className="spine-rule-aside" data-warn={held || undefined}>
									{stateWord(send)}
									{held ? "" : ` · ${timeOf(movedAt(send))}`}
								</span>
							}
						/>
						{send.work !== undefined && (
							<div className="inbox-line">
								<WorkRow item={send} live={working.has(send.work.key)} onOpen={() => onOpenThread(send.to, send.work!, send.title)} />
							</div>
						)}
						{items.map((item) => {
							const drawn = said(item, before);
							before = item.from;
							return drawn;
						})}
						{held ? (
							<p className="inbox-lane-note">
								Waiting for your approval.{" "}
								<button type="button" className="inbox-go" onClick={() => onGoToCard(send.to)}>
									Go to the card
								</button>
							</p>
						) : (
							items.length === 0 && <p className="inbox-lane-note">No reply yet.</p>
						)}
					</div>
				);
			})}
		</>
	) : (
		one.items.map((item, at) => said(item, one.items[at - 1]?.from))
	);
	return (
		<section ref={page} className="dock-page inbox-topic" aria-label={`Messages: ${one.title}`}>
			<Band>
				<button type="button" className="control btn-icon pane-back" title="Back" aria-label="Back" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				<BandKind kind={INBOX_KIND} state={word} />
				<PaneClose onClose={onClose} />
			</Band>
			<Scroll>
				<PaneHead title={one.title} line={<StateLine word={word} detail={detail} time={timeOf(one.moved)} you={!group && one.held} />} />
				<SectionRule
					title={group ? "Sent" : "Messages"}
					status={statusOf(one)}
					{...(group ? { aside: <span className="spine-rule-aside">to {one.sends.length} teammates</span> } : { count: one.items.length })}
				/>
				<div className="inbox-stream">{stream}</div>
				<div className="inbox-foot">
					<button type="button" className="control btn btn-sm" title={`Ask ${nameOf(one.owner)}`} onClick={() => onFollowUp({ aboutItem: one.items.at(-1)!.id, quote: `${one.title} · ${others}` })}>
						Ask a follow-up
					</button>
				</div>
			</Scroll>
		</section>
	);
}

/**
 * One message of a topic: a bubble when it is short and a letter when it
 * is long, on the owner's side or the other's; under it the time (a
 * letter has its own) and one word for where it stands, which opens its
 * history in place; the work thread it started, live while it runs.
 */
function Message({
	item,
	topic: one,
	nameOf,
	named,
	history,
	live,
	onHistory,
	onOpenThread,
	onGoToCard,
}: {
	item: InboxItem;
	topic: Topic;
	nameOf(personaId: string): string;
	/** The first of a run of one sender's messages, which says whose they are. */
	named: boolean;
	history: boolean;
	live: boolean;
	onHistory(): void;
	onOpenThread(owner: string, thread: ThreadId, title: string): void;
	onGoToCard(to: string): void;
}) {
	const short = isShort(item);
	const held = item.held === true && item.state !== "closed";
	const word = stateWord(item);
	const rows = history ? receiptRows(item) : [];
	return (
		<div className="inbox-msg" data-side={item.from === one.owner ? "right" : "left"} data-message={item.id}>
			<div className="inbox-line">
				{short ? (
					<>
						{named && <p className="inbox-who">{item.notice !== undefined ? "Hotline" : nameOf(item.from)}</p>}
						<div className="inbox-bubble selectable">{bubbleText(item)}</div>
					</>
				) : (
					<Letter item={item} topic={one} nameOf={nameOf} />
				)}
				<p className="inbox-meta">
					{short && (
						<>
							<span>{timeOf(item.ts)}</span>
							<span>·</span>
						</>
					)}
					<button type="button" className="inbox-state" data-warn={held || undefined} aria-expanded={history} title="History" onClick={onHistory}>
						{word}
					</button>
					{held && (
						<>
							<span>·</span>
							<button type="button" className="inbox-go" onClick={() => onGoToCard(item.to)}>
								Go to the card
							</button>
						</>
					)}
				</p>
				{item.work !== undefined && <WorkRow item={item} live={live} onOpen={() => onOpenThread(item.to, item.work!, item.title)} />}
			</div>
			{history && (
				<ol className="inbox-receipts spine-list inbox-history">
					{rows.map((row, at) => (
						<li key={`${row.at}:${row.word}`}>
							<Mark kind={row.warn === true ? "waiting" : at === rows.length - 1 && item.state !== "closed" ? "waiting" : "done"} blocked={row.warn === true} />
							<span className="inbox-receipt-word" data-warn={row.warn}>
								{row.word}
							</span>
							{row.detail !== undefined && <span className="inbox-receipt-detail selectable">{row.detail}</span>}
							<span className="inbox-receipt-at">{row.warn === true ? "" : timeOf(row.at)}</span>
						</li>
					))}
				</ol>
			)}
		</div>
	);
}

/** A long message: who and when at its head, its own title where it is not the topic's or a reply's, then six lines and Show all. */
function Letter({ item, topic: one, nameOf }: { item: InboxItem; topic: Topic; nameOf(personaId: string): string }) {
	const subject = item.title !== one.title && !item.title.startsWith("Re: ");
	return (
		<article className="inbox-letter">
			<header className="inbox-letter-head">
				<span>{item.notice !== undefined ? "Hotline" : nameOf(item.from)}</span>
				<span className="inbox-letter-at">{timeOf(item.ts)}</span>
			</header>
			{subject && <p className="inbox-subject">{item.title}</p>}
			<Clamped watch={item.body}>
				<Markdown text={item.body} />
			</Clamped>
		</article>
	);
}

/** The work thread a message started, as a row under it that opens the thread, live while it runs. */
function WorkRow({ item, live, onOpen }: { item: InboxItem; live: boolean; onOpen(): void }) {
	const state = live ? "Running" : item.state === "closed" ? "Done" : "Open";
	return (
		<button type="button" className="inbox-work" data-work={item.id} data-live={live || undefined} aria-label={`Work thread: ${item.title}, ${state}. Open it`} onClick={onOpen}>
			<span className="inbox-work-mark" aria-hidden="true">
				≡
			</span>
			<span>Work thread · {state}</span>
			{live && <span aria-hidden="true" className="dock-dot beat" data-state="running" />}
			<ChevronRightIcon />
		</button>
	);
}
