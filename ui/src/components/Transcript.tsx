import { ErrorCard } from "./ErrorCard";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { DeliveryCause, PlanEntry, ThreadId, TranscriptEvent } from "../generated/contract";
import { type Block, type CallLink, type ScheduledEvent, type Step, groupCalls, groupScheduled } from "../scheduledRuns";
import { ArrowDownIcon, CheckIcon, ChevronDownIcon, ChevronRightIcon, ClockIcon, WarningIcon } from "../icons";
import { dmOf, type Streaming } from "../tape";
import { handedIn, handoffsUnderway, linkFailed, linkLine, namesSaid, threadOfLink } from "../links";
import { activityOf } from "../activity";
import { Glyph } from "../ui/Glyph";
import type { Person } from "../avatars";
import { Avatar } from "../ui/Avatar";
import { Scroll } from "../ui/Scroll";
import { failedLast } from "../retryTurn";
import { wire } from "../wire";
import {
	deliveryLine,
	deliveryMissed,
	foldReactions,
	peerLine,
	quotedLine,
	retryForNotice,
	type Run,
	type Side,
	sideOf,
	superseded,
	toBlocks,
	withStreaming,
} from "../transcriptFolds";
import { useCadence, useLanding, useScrollToEvent, useSleep, useSteady } from "./transcript/hooks";
import { CallTools, Steps } from "./transcript/Steps";
import { AgentSay, NamedSay, UserBubble, type ReactTarget, type ReplyTarget } from "./transcript/Bubbles";
import { ExchangePaused, HumanAction, PasskeyAskCard, Permission } from "./transcript/Asks";
import { ComputerFrame } from "./transcript/ComputerFrame";

/* What the transcript's importers read from here, wherever it now lives. */
export {
	deliveryLine,
	deliveryMissed,
	foldReactions,
	peerLine,
	reactionQuote,
	retryForNotice,
	runEnding,
	stepRuns,
	stepsSummary,
	superseded,
	turnCauseLine,
} from "../transcriptFolds";
export { StepRows } from "./transcript/Steps";
export { SaidAloud, type ReactTarget, type ReplyTarget } from "./transcript/Bubbles";

/** Long enough that a stamp means "we picked this back up later". */
const STAMP_AFTER = 20 * 60_000;

/** How near the top a scroll asks for the page above. */
const EARLIER_SLACK = 600;

/** Slack under the latest line that still counts as following the conversation. */
const PIN_SLACK = 80;

/** Whose chair we are in, for a peer thread: this teammate is `mine`. */
export type Speakers = { me: string; them: string; mine: "user" | "agent" };

/**
 * The conversation, and only the conversation.
 *
 * Drawn the way a messages app draws a 1:1: their words in bubbles on the
 * left, yours on the right, a run from one side tightening its corners.
 * Nothing streams. A friend's reply arrives whole; while it is on its way
 * the mark floats above the composer, at work, with one word for what kind
 * of work. The machinery an agent runs on is folded, not hidden: what
 * happened between two messages is one quiet caption with a count, and any
 * row in it opens on a press. A transcript that lies about what happened is
 * not worth having.
 *
 * Imported tapes also hold permission cards, plans, peer markers, hands-to-
 * human and computer frames. A card with no decision is live: answering it
 * writes through the tape, so the buttons go away when the room supersedes
 * the line.
 */
export function Transcript({
	personaId,
	name,
	avatarHash,
	people,
	events,
	streaming,
	live,
	cornered = false,
	workingCalls,
	focus,
	speakers,
	onReply,
	onReact,
	onRetryMessage,
	onOpenThread,
	onOpenThreads,
	onOpenScreen,
	onOpenWork,
	workOpen,
	onProviders,
	backendId,
	onChooseAccount,
	thread,
	more = false,
	onEarlier,
}: {
	personaId: string;
	/** Which thread this is, so a card is answered to it; the teammate's main conversation when unset. */
	thread?: ThreadId;
	name: string;
	/** The teammate's picture, when it has one. */
	avatarHash?: string | undefined;
	/** Every teammate by id, for the colleagues a line names. */
	people?: ReadonlyMap<string, Person> | undefined;
	events: TranscriptEvent[];
	streaming: Streaming[];
	/** A turn is running: the mark is up, above the composer. */
	live: boolean;
	/**
	 * Something stands at the composer's top right, over the foot of the
	 * conversation: the last line keeps clear of it as it does of the mark,
	 * and the way back down sits above it.
	 */
	cornered?: boolean;
	/** Call thread IDs with a turn in flight, from the roster. */
	workingCalls?: readonly string[] | undefined;
	/** A search hit to land on. `at` is a nonce so picking the same id twice still jumps. */
	focus: { eventId: string; at: number } | null;
	/** A peer thread, or a handoff, names both sides; the tape with the person does not. */
	speakers?: Speakers;
	onReply?(target: ReplyTarget): void;
	/** An emoji on a teammate's line, sent the way the phone sends one. */
	onReact?(target: ReactTarget, emoji: string): void;
	onRetryMessage?(message: Extract<TranscriptEvent, { kind: "user" }>): void;
	/** Opens a thread a line stands for in the right-hand pane: a link, a peer line or a delivery. */
	onOpenThread?(thread: ThreadRef): void;
	/**
	 * Opens the teammate's threads in the pane. Given only for the person's
	 * own conversation with it, which keeps one line for work colleagues
	 * handed it rather than a line each.
	 */
	onOpenThreads?(): void;
	/** The teammate's desktop, only while one is running: opens it in a window of its own. */
	onOpenScreen?(): void;
	/**
	 * Opens a turn's work in the pane beside the conversation: a run of
	 * steps by its block id, or `null` for the turn running now. Without it
	 * the steps open in place, as a thread or a run draws them.
	 */
	onOpenWork?(blockId: string | null): void;
	/** What the work pane is showing, so its caption reads as open. */
	workOpen?: string | null | undefined;
	/** Opens Settings › Providers, from a card saying no provider is connected. */
	onProviders?(): void;
	backendId?: string;
	onChooseAccount?(): void;
	/** Older lines are on the desk, above what is loaded. */
	more?: boolean;
	/** Loads them: a page, or back as far as one line. */
	onEarlier?(through?: string): Promise<void>;
}) {
	const scroller = useRef<HTMLDivElement>(null);
	/* Following the conversation is the default and stays true until you
	 * scroll away from the bottom yourself. */
	const pinned = useRef(true);
	/* The scroll the code is about to cause by pinning, so the scrollbar does
	 * not show itself for a move you did not make. */
	const pinning = useRef(false);
	/* The same fact, for the button that offers the way back down. */
	const [following, setFollowing] = useState(true);
	const empty = events.length === 0 && !live;
	/* Pressing the mark opens the work behind it for this turn. It closes
	 * again when the reply lands: what you asked to watch was the turn, not
	 * the transcript. */
	const [workShown, setWorkShown] = useState(false);
	useEffect(() => {
		if (!live) setWorkShown(false);
	}, [live]);
	/* A reply quote is the same jump as a search hit, asked for from inside
	 * the transcript rather than from the search. The later `at` wins. */
	const [jumped, setJumped] = useState<{ eventId: string; at: number } | null>(null);
	const landing = jumped !== null && (focus === null || jumped.at > focus.at) ? jumped : focus;

	/* Everything derived from the written events is worked out once per
	 * change to them, not once per streamed word: a reply streams into a
	 * chapter that can hold hundreds of rows, and only its own bubble moves. */
	const said = useMemo(() => {
		const lines = new Map<string, string>();
		for (const event of events) {
			const line = quotedLine(event);
			if (line !== undefined) lines.set(event.id, line);
		}
		return lines;
	}, [events]);
	const answered = useMemo(() => superseded(events), [events]);
	const reacted = useMemo(() => foldReactions(events), [events]);
	const written = useMemo(() => toBlocks(events), [events]);
	const onJump = useCallback((eventId: string) => setJumped({ eventId, at: Date.now() }), []);
	/* A row is skipped while its props are the same, so the thread it is
	 * handed is made once, not once per render. */
	const rowThread = useMemo(() => thread ?? dmOf(personaId), [thread, personaId]);
	/* One retry per notice for as long as the events are these; a fresh
	 * function each render would wake the row it is given to. */
	const retries = useMemo(() => {
		const made = new Map<string, (() => void) | undefined>();
		return (noticeId: string) => {
			if (!made.has(noticeId) && onRetryMessage !== undefined) made.set(noticeId, retryForNotice(events, noticeId, onRetryMessage));
			return made.get(noticeId);
		};
	}, [events, onRetryMessage]);

	/* The teammate's own conversation can run its failed turn again; a thread's cannot. */
	const runAgain = useCallback(() => wire.command("session.retry", { personaId }), [personaId]);

	useScrollToEvent(scroller, pinned, landing, events);

	/* The window opens on the tape's last lines. Nearing the top loads the
	 * page above, and the rows you were reading stay where they were: the
	 * column grows upward by exactly what arrived. */
	const earlierRef = useRef<{ more: boolean; load?: ((through?: string) => Promise<void>) | undefined }>({ more });
	earlierRef.current = { more, load: onEarlier };
	const anchor = useRef<{ height: number; top: number; first: string | undefined } | null>(null);
	const firstShown = useRef(events[0]?.id);
	firstShown.current = events[0]?.id;
	const loadEarlier = useCallback((through?: string) => {
		const { more, load } = earlierRef.current;
		const el = scroller.current;
		if (!more || load === undefined || !el || anchor.current !== null) return;
		const taken = { height: el.scrollHeight, top: el.scrollTop, first: firstShown.current };
		anchor.current = taken;
		void load(through).finally(() => {
			// The page lands as a render that may not have committed when the
			// load settles; a frame later it has, and the layout effect below
			// has let go of the anchor. One still held means nothing arrived:
			// let the next scroll ask again.
			requestAnimationFrame(() => {
				if (anchor.current === taken) anchor.current = null;
			});
		});
	}, []);
	useLayoutEffect(() => {
		const el = scroller.current;
		const was = anchor.current;
		if (!el || was === null || events[0]?.id === was.first) return;
		anchor.current = null;
		if (!pinned.current) el.scrollTop = was.top + (el.scrollHeight - was.height);
	}, [events]);
	// A search hit or a quoted reply above the window: load back to it, and
	// the jump lands once it is there.
	useEffect(() => {
		if (landing === null || !more || events.some((one) => one.id === landing.eventId)) return;
		loadEarlier(landing.eventId);
	}, [landing, more, events, loadEarlier]);
	const loadEarlierRef = useRef(loadEarlier);
	loadEarlierRef.current = loadEarlier;

	// biome-ignore lint/correctness/useExhaustiveDependencies: the scroller mounts and unmounts with empty, and the listeners move with it.
	useEffect(() => {
		const el = scroller.current;
		if (!el) return;
		// The layout the last decision was taken on. A scroll event lands a
		// frame after the scroll that caused it, and while the window is being
		// resized every frame reflows the bubbles: the pin's own scroll arrives
		// to a column hundreds of pixels taller or shorter than the one it
		// pinned, and read as distance it looks like you scrolling away. So a
		// scroll that comes with a new layout never lets go of the bottom; only
		// one on the layout already seen can.
		let seen = { height: el.scrollHeight, view: el.clientHeight };
		const measure = () => {
			if (pinned.current && (el.scrollHeight !== seen.height || el.clientHeight !== seen.view)) {
				pin();
				return;
			}
			seen = { height: el.scrollHeight, view: el.clientHeight };
			pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < PIN_SLACK;
			setFollowing(pinned.current);
			if (el.scrollTop < EARLIER_SLACK) loadEarlierRef.current();
		};
		const pin = () => {
			seen = { height: el.scrollHeight, view: el.clientHeight };
			if (!pinned.current) return;
			const was = el.scrollTop;
			el.scrollTop = el.scrollHeight;
			// Only a scroll that really moves fires an event to be told apart.
			if (el.scrollTop !== was) pinning.current = true;
		};
		// A scroll, a reflow and a resize can all land in one frame, and each
		// would read the layout and write the position again; they are done
		// once, on the frame.
		let frame = 0;
		let scrolled = false;
		let resized = false;
		const settle = () => {
			frame = 0;
			if (resized) pin();
			if (scrolled) measure();
			scrolled = false;
			resized = false;
		};
		const ask = () => {
			if (frame === 0) frame = requestAnimationFrame(settle);
		};
		const onScroll = () => {
			scrolled = true;
			ask();
		};
		const onResize = () => {
			resized = true;
			ask();
		};
		el.addEventListener("scroll", onScroll, { passive: true });
		// Markdown lays out after the event lands, so the column's height
		// changes without a scroll event; this is what notices.
		const observer = new ResizeObserver(onResize);
		observer.observe(el);
		if (el.firstElementChild) observer.observe(el.firstElementChild);
		pin();
		return () => {
			el.removeEventListener("scroll", onScroll);
			observer.disconnect();
			cancelAnimationFrame(frame);
		};
		// The scroll listener above is enough while the events are the same.
	}, [empty]);

	// Hooks before the empty-state return, so their order never changes.
	const arrived = useMemo(() => withStreaming(written, streaming), [written, streaming]);
	const hidden = useCadence(personaId, arrived);
	const queued = hidden.size > 0;
	const currentActivity = useMemo(() => live || queued ? activityOf(events, streaming, queued) : null, [live, queued, events, streaming]);
	const { activity, sleeping } = useSleep(personaId, useLanding(personaId, useSteady(currentActivity)));

	// A thread is said once: as its answer when one came back, else as its
	// marker. Your own answer to a card is on the card, so it draws no row.
	// A job that fired again and again with nothing drawn between is one row,
	// so this is decided on what is left to draw.
	const blocks = useMemo(() => groupCalls(groupScheduled(
		arrived.filter(
			(block) =>
				!(
					block.kind === "event" &&
					(hidden.has(block.event.id) ||
						answered.has(block.event.id) ||
						reacted.lines.has(block.event.id) ||
						(block.event.kind === "delivery" && block.event.cause.kind === "answer") ||
						(onOpenThreads !== undefined && block.event.kind === "link" && handedIn(block.event, personaId)))
				),
		),
	)), [arrived, hidden, answered, reacted, onOpenThreads, personaId]);
	const underway = useMemo(() => onOpenThreads === undefined ? { count: 0, from: [] } : handoffsUnderway(events, personaId), [onOpenThreads, events, personaId]);
	// Which side each block speaks from, with the machinery between two
	// messages transparent, so two agent lines around a tool call are still
	// one run of speech.
	const sides = useMemo(() => blocks.map(sideOf), [blocks]);
	const sideBefore = (index: number): Side => {
		for (let at = index - 1; at >= 0; at--) if (sides[at] !== null) return sides[at]!;
		return null;
	};
	const sideAfter = (index: number): Side => {
		for (let at = index + 1; at < sides.length; at++) if (sides[at] !== null) return sides[at]!;
		return null;
	};

	if (empty) {
		return (
			<div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 pb-16">
				<Avatar id={personaId} name={name} size={48} hash={avatarHash} />
				<p className="text-lg font-semibold">{name}</p>
				<p className="text-center text-sm text-ink-3">Nothing said yet. Say hello below.</p>
			</div>
		);
	}

	return (
		<div className="relative flex min-h-0 flex-1 flex-col">
		<Scroll scrollerRef={scroller} programmatic={pinning}>
			{/* `justify-end` rests a short conversation on the composer rather
			    than stranding it at the top of an empty pane. The room at the
			    bottom is the mark's for as long as the mark is there — through
			    the landing and the sleep, not just the turn — so the last bubble
			    never slides under it. It opens before the mark rises into it
			    (`wake` waits 200ms) and closes once the mark is gone, at once:
			    easing it would reflow every row for the length of the ease.
			    Subagents at the composer's corner keep the same room. */}
			<div
				className={`mx-auto flex min-h-full w-full max-w-[46rem] flex-col justify-end px-6 pt-6 ${activity !== null || cornered ? "pb-14" : "pb-6"}`}
			>
				{/* Scrolling up loads these on its own; the key is for a window
				    whose loaded lines are too short to scroll. */}
				{more && onEarlier !== undefined && (
					<button type="button" className="control btn btn-sm mb-3 self-center" onClick={() => loadEarlier()}>
						Earlier messages
					</button>
				)}
				{blocks.map((block, index) => {
					const previous = blocks[index - 1];
					const stamp =
						!isChapter(block) &&
						(previous === undefined || (!isChapter(previous) && block_ts(block) - block_ts(previous) > STAMP_AFTER));
					const id = block.kind === "event" ? block.event.id : block.id;
					const side = sides[index] ?? null;
					const run: Run = {
						top: side !== null && !stamp && sideBefore(index) === side,
						bottom: side !== null && sideAfter(index) === side,
					};
					return (
						<div key={id} data-event-id={id} className="tape-row">
							{stamp && <p className="rule-line rule-line-plain">{stampText(block_ts(block))}</p>}
							{block.kind === "scheduled" ? (
								<ScheduledGroup name={block.name} runs={block.runs} />
							) : block.kind === "calls" ? (
								<CallGroup calls={block.calls} personaId={personaId} people={people} onOpen={(link) => onOpenThread?.({ thread: threadOfLink(link), title: link.title })} />
							) : block.kind === "steps" ? (thread?.kind === "call" ? (
								<CallTools items={block.items} />
							) : (
								<Steps
									id={block.id}
									items={block.items}
									live={live && index === blocks.length - 1}
									shown={workShown}
									{...(onOpenWork !== undefined ? { onOpenWork, open: workOpen === block.id } : {})}
								/>
							)) : (
								<Row
									personaId={personaId}
									thread={rowThread}
									ownerName={name}
									event={block.event}
									working={block.event.kind === "link" && block.event.threadKind === "call" && (workingCalls?.includes(block.event.thread) ?? false)}
									quote={block.event.kind === "user" && block.event.replyTo !== undefined ? said.get(block.event.replyTo) : undefined}
									top={run.top}
									bottom={run.bottom}
									speakers={speakers}
									people={people}
									{...(onRetryMessage && !speakers && block.event.kind === "notice" ? { onRetry: retries(block.event.id) } : {})}
									{...(!speakers && thread === undefined && block.event.kind === "notice" && failedLast(events, block.event.id) ? { onRunAgain: runAgain } : {})}
									reactions={reacted.on.get(block.event.id)}
									{...(onReply !== undefined ? { onReply } : {})}
									{...(onReact !== undefined ? { onReact } : {})}
									{...(onOpenThread !== undefined ? { onOpenThread } : {})}
									{...(onOpenScreen !== undefined ? { onOpenScreen } : {})}
									{...(onProviders !== undefined ? { onProviders } : {})}
									{...(!speakers && thread === undefined && backendId !== undefined ? { backendId } : {})}
									{...(!speakers && thread === undefined && onChooseAccount !== undefined ? { onChooseAccount } : {})}
									onJump={onJump}
								/>
							)}
						</div>
					);
				})}
				{underway.count > 0 && (
					<button type="button" className="rule-line rule-line-plain mt-3 w-full" onClick={onOpenThreads}>
						<span className="min-w-0 truncate">
							Working on {underway.count === 1 ? "a handoff" : `${underway.count} handoffs`} from {namesSaid(underway.from)}
						</span>
						<span className="shrink-0">· Open</span>
					</button>
				)}
			</div>
		</Scroll>
		{activity !== null && (
			<div className="ambient">
				<div className="mx-auto w-full max-w-[46rem] px-6">
					<button
						type="button"
						className="ambient-mark"
						data-sleeping={sleeping || undefined}
						aria-expanded={onOpenWork !== undefined ? workOpen === null : workShown}
						title={(onOpenWork !== undefined ? workOpen === null : workShown) ? "Hide the work" : "Show the work"}
						onClick={() => (onOpenWork !== undefined ? onOpenWork(null) : setWorkShown((was) => !was))}
					>
						<Glyph phase={activity.phase} />
						{activity.word !== "" && <span className="ambient-word">{activity.word}</span>}
					</button>
				</div>
			</div>
		)}
		{!following && (
			<button
				type="button"
				className={`control btn send jump-latest absolute right-8 ${cornered ? "bottom-12" : "bottom-3"}`}
				title="Jump to the latest"
				aria-label="Jump to the latest"
				onClick={() => {
					const el = scroller.current;
					if (!el) return;
					pinned.current = true;
					el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
				}}
			>
				<ArrowDownIcon />
			</button>
		)}
		</div>
	);
}

function block_ts(block: Block): number {
	return block.kind === "event" ? block.event.ts : block.ts;
}

function isChapter(block: Block): boolean {
	return block.kind === "event" && block.event.kind === "chapter";
}

/**
 * One written row. Memoized: while a reply streams, every row above it gets
 * the same props and is skipped, so the cost of a word is the live bubble's.
 */
const Row = memo(function Row({
	personaId,
	thread,
	ownerName,
	event,
	working,
	quote,
	top,
	bottom,
	speakers,
	people,
	reactions,
	onReply,
	onReact,
	onRetry,
	onRunAgain,
	onOpenThread,
	onOpenScreen,
	onProviders,
	backendId,
	onChooseAccount,
	onJump,
}: {
	personaId: string;
	thread: ThreadId;
	/** Whose tape this is: the teammate, so a card can speak of it in the third person. */
	ownerName: string;
	event: Exclude<TranscriptEvent, Step>;
	working: boolean;
	/** What the message this one replies to said, when it is on the tape. */
	quote: string | undefined;
	top: boolean;
	bottom: boolean;
	speakers: Speakers | undefined;
	people: ReadonlyMap<string, Person> | undefined;
	/** Emoji the phone or this window sent as lines of their own, folded onto this one. */
	reactions: string[] | undefined;
	onRetry?: (() => void) | undefined;
	/** Runs the failed turn this notice reports again (`session.retry`). */
	onRunAgain?: (() => Promise<unknown>) | undefined;
	onReply?(target: ReplyTarget): void;
	onReact?(target: ReactTarget, emoji: string): void;
	onOpenThread?(thread: ThreadRef): void;
	onOpenScreen?(): void;
	onProviders?(): void;
	backendId?: string;
	onChooseAccount?(): void;
	onJump(eventId: string): void;
}) {
	const run: Run = { top, bottom };
	const own = event.kind === "user" || event.kind === "agent" ? event.reactions : undefined;
	const worn = reactions === undefined ? own : [...(own ?? []), ...reactions];
	if ((event.kind === "permission" || event.kind === "human_action" || event.kind === "passkey_ask") && event.from?.kind === "call") {
		const origin = event.from;
		const settled = event.kind === "permission" ? event.decision !== undefined : event.status !== "pending";
		const label = event.kind === "permission" ? event.title : event.kind === "human_action" ? event.reason : event.name;
		if (thread.kind === "call") return (
			<button type="button" className="rule-line rule-line-plain"
				onClick={() => onOpenThread?.({ thread: { kind: "dm", key: personaId }, eventId: event.id })}>
				{settled ? "Settled" : "Needs you"}: {label} · Open card in home chat
			</button>
		);
		return (
			<div>
				<button type="button" className="said-name"
					onClick={() => onOpenThread?.({ thread: { kind: "call", key: origin.thread }, ...(origin.title ? { title: origin.title } : {}) })}>
					From the call{origin.title ? ` · ${origin.title}` : ""}
				</button>
				{event.kind === "permission" && <Permission thread={thread} event={event} />}
				{event.kind === "human_action" && <HumanAction thread={thread} event={event} {...(onOpenScreen !== undefined ? { onOpenScreen } : {})} />}
				{event.kind === "passkey_ask" && <PasskeyAskCard personaId={personaId} event={event} />}
			</div>
		);
	}
	switch (event.kind) {
		case "user":
			if (thread.kind === "call") return <>
                <p className="said-name text-right">You</p>
                <UserBubble event={event} quote={quote} run={run} reactions={worn} onJump={onJump} {...(onReply !== undefined ? { onReply } : {})} />
            </>;
			return event.scheduled !== undefined ? (
				<ScheduledLine name={event.scheduled.name} prompt={event.text} />
			) : speakers !== undefined ? (
				<NamedSay name={speakers.mine === "user" ? speakers.me : speakers.them} mine={speakers.mine === "user"} text={event.text} />
			) : (
				<UserBubble event={event} quote={quote} run={run} reactions={worn} onJump={onJump} {...(onReply !== undefined ? { onReply } : {})} />
			);

		case "agent":
            if (thread.kind === "call") {
                const { spoken, ...written } = event;
                const heard = spoken?.trim() ? spoken : event.text;
                return <>
                    <p className="said-name">{ownerName}</p>
                    <AgentSay personaId={personaId} event={{ ...written, text: heard }} run={run} reactions={worn}
                        {...(onReply !== undefined ? { onReply } : {})} {...(onReact !== undefined ? { onReact } : {})}
                        {...(onOpenThread !== undefined ? { onOpenThread } : {})} />
                    {heard !== event.text && <details className="my-2 text-sm text-ink-3">
                        <summary>Written reply</summary>
                        <NamedSay name={ownerName} mine={false} text={event.text} />
                    </details>}
                </>;
            }
			return speakers !== undefined ? (
				<NamedSay
					name={speakers.mine === "agent" ? speakers.me : speakers.them}
					mine={speakers.mine === "agent"}
					text={event.text}
				/>
			) : (
				<AgentSay
                    {...(onOpenThread !== undefined ? { onOpenThread } : {})}
					personaId={personaId}
					event={event}
					run={run}
					reactions={worn}
					{...(onReply !== undefined ? { onReply } : {})}
					{...(onReact !== undefined ? { onReact } : {})}
				/>
			);

		/* Where the turn stopped. Drawn only when the stop was not the agent's
		 * own choice; a count of tokens is the harness's business, not the
		 * conversation's. */
		case "turn":
			if (event.stopReason === "end_turn") return null;
			return <p className="instrument mt-1 text-right text-ink-4">{event.stopReason.replace(/_/g, " ")}</p>;

		case "notice":
			if (event.level === "error")
				return (
					<ErrorCard
						text={event.text}
						at={event.ts}
						{...(!speakers ? { personaId } : {})}
						{...(onRetry ? { onRetry } : {})}
						{...(onRunAgain ? { onRunAgain } : {})}
						{...(onProviders ? { onProviders } : {})}
						{...(backendId !== undefined ? { backendId, teammate: ownerName } : {})}
						{...(onChooseAccount ? { onChooseAccount } : {})}
					/>
				);
			return (
				<p
					className="rule-line rule-line-plain gap-1.5"
					style={{ color: event.level === "info" ? "var(--ink-3)" : event.level === "warn" ? "var(--warn)" : "var(--danger)" }}
				>
					{event.level !== "info" && <WarningIcon className="shrink-0" />}
					<span className="selectable font-normal">{event.text}</span>
				</p>
			);

		/* Where the agent's working context reset: a line across the column
		 * with the chapter's name on it once it has one. The close arrives as
		 * one superseded marker, so there is no interim state to draw. */
		case "chapter":
			return (
				<p className="rule-line mt-2">
					<span className="max-w-[70%] truncate text-ink-2">
						{event.title !== undefined && event.title !== "" ? event.title : "New chapter"}
					</span>
					<span>{stampText(event.ts)}</span>
				</p>
			);

		case "permission":
			return <Permission thread={thread} event={event} />;

		case "plan":
			return <Plan entries={event.entries} />;

		case "human_action":
			return <HumanAction thread={thread} event={event} {...(onOpenScreen !== undefined ? { onOpenScreen } : {})} />;

		case "passkey_ask":
			return <PasskeyAskCard personaId={personaId} event={event} />;

		/* The pair's exchange hit its cap: a live card while it waits on the
		 * person, a quiet rule line once it is settled either way. */
		case "exchange_paused":
			return <ExchangePaused personaId={personaId} ownerName={ownerName} event={event} />;

		/* A thread with a colleague while it has no answer to show: hung
		 * under the row before it, the way a reply's steps are, so the
		 * conversation reads the same either side of the answer arriving.
		 * Pressing it opens the thread in the inspector's place. */
		case "peer":
			return (
				<button type="button" className="hung-line" data-missed={event.status === "failed" || undefined} onClick={() => onOpenThread?.({ thread: { kind: "pair", key: event.threadKey }, withName: event.withName })}>
					<Avatar id={event.withPersonaId} name={event.withName} size={16} hash={people?.get(event.withPersonaId)?.hash} />
					<span className="min-w-0 truncate">{peerLine(event)}</span>
					<ChevronRightIcon />
				</button>
			);

		/* A colleague's answer or handoff, in their voice: their face, their
		 * name and the start of what they said, hung under the row before
		 * it. It opens the originating thread, with the handoff's request and
		 * reply route. Your own answer never gets here: it is on its card. */
		case "delivery": {
			const cause = event.cause;
			const line = deliveryLine(event);
			if (cause.kind === "answer" || line === null) return null;
			// Where both sides are named, a colleague's words are the conversation: said whole, in their voice.
			if (speakers !== undefined) return <NamedSay name={cause.name} mine={false} text={event.text} />;
			return (
				<button
					type="button"
					className="hung-line"
					data-missed={deliveryMissed(event) || undefined}
					onClick={() => onOpenThread?.({
						thread: { kind: "pair", key: cause.threadKey },
						withName: cause.name,
						...(cause.kind === "handoff" ? { handoff: cause } : {}),
					})}
				>
					<Avatar id={cause.personaId} name={line.name} size={16} hash={people?.get(cause.personaId)?.hash} />
					<span className="min-w-0 truncate">
						<span className="hung-line-name">{line.name}</span> {line.said}
					</span>
					<ChevronRightIcon />
				</button>
			);
		}

		/* A thread this conversation holds, whatever its kind: a work thread, a
		 * subagent's run, a call. One quiet line that fills in as the thread
		 * goes, and opens it in the right-hand pane. */
		case "link":
			return (
				<button
					type="button"
					className="rule-line rule-line-plain w-full"
					style={linkFailed(event) ? { color: "var(--warn)" } : undefined}
					aria-label={`${linkLine(event, personaId, people, working)}. Open it`}
					onClick={() => onOpenThread?.({ thread: threadOfLink(event), title: event.title })}
				>
					<span className="min-w-0 truncate">{linkLine(event, personaId, people, working)}</span>
					<span className="shrink-0">· Open</span>
				</button>
			);

		/* The markers links replaced. The window declares `threads2`, so the
		 * desk sends it a link for each, even for one written long ago. */
		case "side":
		case "subagent":
		case "call":
			return null;

		case "computer_frame":
			return <ComputerFrame eventId={event.id} dataUrl={event.dataUrl} />;

		/* Older tapes carry a download's progress; it now lives on the
		 * computer's button, and the conversation never shows it. */
		case "computer_pull":
			return null;
	}
});

/** What opens a thread from a line: the thread, and what the line knew about it. */
export type ThreadRef = {
	eventId?: string;
	thread: ThreadId;
	/** What a link called it. */
	title?: string;
	/** The teammate a pair is with. */
	withName?: string;
	handoff?: Extract<DeliveryCause, { kind: "handoff" }>;
};

/**
 * A line nobody typed: a schedule fired. One row naming the job, with the
 * whole prompt behind a press, because debugging a schedule means reading
 * what it actually said.
 */
function ScheduledLine({ name, prompt }: { name: string; prompt: string }) {
	const [open, setOpen] = useState(false);
	return (
		<div className="mt-3 flex flex-col items-end">
			<button type="button" className="step w-auto max-w-[78%]" aria-expanded={open} onClick={() => setOpen((was) => !was)}>
				<ClockIcon className="shrink-0 text-ink-3" />
				<span className="min-w-0 truncate">
					<span className="text-ink-3">Scheduled · </span>
					{name}
				</span>
				{open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />}
			</button>
			{open && <div className="speech said-me mt-1">{prompt}</div>}
		</div>
	);
}

/**
 * A job that fired again and again with nothing said between: one row that
 * counts the runs. Pressed, it lists when each ran, oldest first, and shows
 * the latest prompt, which is the one that is still in force.
 */
function ScheduledGroup({ name, runs }: { name: string; runs: ScheduledEvent[] }) {
	const [open, setOpen] = useState(false);
	const latest = runs[runs.length - 1]!;
	return (
		<div className="mt-3 flex flex-col items-end">
			<button
				type="button"
				className="step w-auto max-w-[78%]"
				aria-label={`Scheduled, ${name}, ran ${runs.length} times`}
				aria-expanded={open}
				onClick={() => setOpen((was) => !was)}
			>
				<ClockIcon className="shrink-0 text-ink-3" />
				<span className="flex min-w-0">
					<span className="shrink-0 text-ink-3">Scheduled · </span>
					<span className="truncate">{name}</span>
					<span className="shrink-0 text-ink-3"> · ran {runs.length} times</span>
				</span>
				{open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />}
			</button>
			{open && (
				<>
					<ul className="instrument mt-1 flex flex-col items-end">
						{runs.map((run) => (
							<li key={run.id}>{runTime(run.ts)}</li>
						))}
					</ul>
					<div className="speech said-me mt-1">{latest.text}</div>
				</>
			)}
		</div>
	);
}

/**
 * Calls one after another with nothing said between: one quiet line that
 * counts them and their time on the line. Pressed, it lists each call's own
 * line, oldest first, each opening that call.
 */
function CallGroup({
	calls,
	personaId,
	people,
	onOpen,
}: {
	calls: CallLink[];
	personaId: string;
	people: ReadonlyMap<string, Person> | undefined;
	onOpen(link: CallLink): void;
}) {
	const [open, setOpen] = useState(false);
	const minutes = Math.round(calls.reduce((sum, link) => sum + ((link.at ?? link.ts) - link.ts), 0) / 60_000);
	const words = `${calls.length} calls · ${minutes < 1 ? "under a minute" : `${minutes} min`} in all`;
	return (
		<div className="flex flex-col">
			<button type="button" className="rule-line rule-line-plain w-full" aria-expanded={open} aria-label={`${words}. ${open ? "Hide" : "Show"} them`} onClick={() => setOpen((was) => !was)}>
				<span className="min-w-0 truncate">{words}</span>
				<span className="shrink-0">· {open ? "Hide" : "Show"}</span>
			</button>
			{open &&
				calls.map((link) => (
					<button
						key={link.id}
						type="button"
						className="rule-line rule-line-plain w-full"
						aria-label={`${linkLine(link, personaId, people)}. Open it`}
						onClick={() => onOpen(link)}
					>
						<span className="min-w-0 truncate">{linkLine(link, personaId, people)}</span>
						<span className="shrink-0">· Open</span>
					</button>
				))}
		</div>
	);
}

/** The agent's working list, one status per line. */
function Plan({ entries }: { entries: PlanEntry[] }) {
	if (entries.length === 0) return null;
	return (
		<ul className="mt-2 max-w-[78%] py-1 text-sm">
			{entries.map((entry, index) => (
				<li key={`${index}:${entry.content}`} className="flex items-start gap-2 py-0.5 pl-2">
					<PlanMark status={entry.status} />
					<span className={`min-w-0 flex-1 ${entry.status === "completed" ? "text-ink-3" : "text-ink-2"}`}>
						{entry.content}
					</span>
				</li>
			))}
		</ul>
	);
}

function PlanMark({ status }: { status: string }) {
	if (status === "completed") {
		return <CheckIcon className="mt-px shrink-0 text-accent-ink" />;
	}
	if (status === "in_progress") {
		return (
			<span className="grid h-4 w-4 shrink-0 place-items-center" aria-label="in progress">
				<span className="beat h-1.5 w-1.5 rounded-full bg-accent" />
			</span>
		);
	}
	return (
		<span className="grid h-4 w-4 shrink-0 place-items-center" aria-label={status}>
			<span className="h-1.5 w-1.5 rounded-full" style={{ boxShadow: "inset 0 0 0 1.5px var(--ink-4)" }} />
		</span>
	);
}

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

const shortDate = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short", month: "short", day: "numeric" });

function stampText(at: number): string {
	const when = new Date(at);
	const days = daysBetween(when, new Date());
	if (days === 0) return `Today ${clock.format(when)}`;
	if (days === 1) return `Yesterday ${clock.format(when)}`;
	return `${weekday.format(when)} ${clock.format(when)}`;
}

/** When a run fired: the time, with the date before it when it was not today. */
function runTime(at: number): string {
	const when = new Date(at);
	return daysBetween(when, new Date()) === 0 ? clock.format(when) : `${shortDate.format(when)}, ${clock.format(when)}`;
}

/** Calendar days apart, not elapsed hours: 11pm and 1am are a day apart. */
function daysBetween(then: Date, now: Date): number {
	const midnight = (date: Date) => new Date(date.getFullYear(), date.getMonth(), date.getDate());
	return Math.round((midnight(now).getTime() - midnight(then).getTime()) / 86_400_000);
}
