import { ErrorCard } from "./ErrorCard";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState, type MouseEvent, type RefObject } from "react";
import type {
	Attachment,
	DeliveryCause,
	ExchangePauseStatus,
	HumanActionStatus,
	HumanAnswer,
	PasskeyAskStatus,
	PermissionOption,
	PlanEntry,
	ToolOutput,
	ToolStatus,
	TranscriptEvent,
} from "../generated/contract";
import { chordKeys } from "../chords";
import { REST, step, type Bubble, type Cadence } from "../cadence";
import { bubbleId, pacedLive } from "../pacing";
import { wholeBubbles } from "../reveal";
import { type Block, type ScheduledEvent, type Step, groupScheduled } from "../scheduledRuns";
import { ArrowDownIcon, CheckIcon, ChevronDownIcon, ChevronRightIcon, ClockIcon, CopyIcon, ReplyIcon, SmileIcon, WarningIcon } from "../icons";
import { popupMessageMenu, writeClipboard } from "../native";
import type { Streaming } from "../tape";
import { type Activity, type ActivityPhase, activityOf, LANDED, RESTING } from "../activity";
import { Glyph, LANDED_MS } from "../ui/Glyph";
import { Avatar } from "../ui/Avatar";
import { Scroll } from "../ui/Scroll";
import { Viewer } from "../ui/Viewer";
import { wire } from "../wire";
import { Markdown } from "./Markdown";
import { askedFor } from "./PasskeyArm";
import { SentFile } from "./SentFile";
import { joinThoughts, outputText, stepTitle } from "../stepText";

/** Long enough that a stamp means "we picked this back up later". */
const STAMP_AFTER = 20 * 60_000;
/** How near the top a scroll asks for the page above. */
const EARLIER_SLACK = 600;
/** Slack under the latest line that still counts as following the conversation. */
const PIN_SLACK = 80;

/** A message being answered: the id the wire stamps, the line the chip shows. */
export type ReplyTarget = { eventId: string; text: string };
/** A line to react to: its id, and all of what it said, for the quote the reaction carries. */
export type ReactTarget = { eventId: string; text: string };

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
	events,
	streaming,
	live,
	focus,
	speakers,
	onReply,
	onReact,
	onRetryMessage,
	onOpenThread,
	onOpenSubagent,
	onOpenSide,
	onOpenScreen,
	onOpenWork,
	workOpen,
	sideId,
	more = false,
	onEarlier,
}: {
	personaId: string;
	/** Set when this is a side thread's transcript: a permission is answered to it, not to the main session. */
	sideId?: string;
	name: string;
	/** The teammate's picture, when it has one. */
	avatarHash?: string | undefined;
	events: TranscriptEvent[];
	streaming: Streaming[];
	/** A turn is running: the mark is up, above the composer. */
	live: boolean;
	/** A search hit to land on. `at` is a nonce so picking the same id twice still jumps. */
	focus: { eventId: string; at: number } | null;
	/** A peer thread names both sides; the tape with the person does not. */
	speakers?: Speakers;
	onReply?(target: ReplyTarget): void;
	/** An emoji on a teammate's line, sent the way the phone sends one. */
	onReact?(target: ReactTarget, emoji: string): void;
	onRetryMessage?(message: Extract<TranscriptEvent, { kind: "user" }>): void;
	onOpenThread?(thread: ThreadRef): void;
	onOpenSubagent?(event: SubagentEvent): void;
	/** Opens a side thread in the right-hand pane from its line. */
	onOpenSide?(event: SideEvent): void;
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
	/** Older lines are on the desk, above what is loaded. */
	more?: boolean;
	/** Loads them: a page, or back as far as one line. */
	onEarlier?(through?: string): Promise<void>;
}) {
	const scroller = useRef<HTMLDivElement>(null);
	/* Following the conversation is the default and stays true until you
	 * scroll away from the bottom yourself. */
	const pinned = useRef(true);
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

	useScrollToEvent(scroller, pinned, landing, events);

	/* The window opens on the tape's last lines. Nearing the top loads the
	 * page above, and the rows you were reading stay where they were: the
	 * column grows upward by exactly what arrived. */
	const earlierRef = useRef<{ more: boolean; load?: ((through?: string) => Promise<void>) | undefined }>({ more });
	earlierRef.current = { more, load: onEarlier };
	const anchor = useRef<{ height: number; top: number; first: string | undefined } | null>(null);
	const loadEarlier = useCallback((through?: string) => {
		const { more, load } = earlierRef.current;
		const el = scroller.current;
		if (!more || load === undefined || !el || anchor.current !== null) return;
		anchor.current = { height: el.scrollHeight, top: el.scrollTop, first: events[0]?.id };
		void load(through).finally(() => {
			// Nothing arrived: let the next scroll ask again.
			if (anchor.current !== null && anchor.current.first === events[0]?.id) anchor.current = null;
		});
	}, [events]);
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
			if (pinned.current) el.scrollTop = el.scrollHeight;
		};
		el.addEventListener("scroll", measure, { passive: true });
		// Markdown lays out after the event lands, so the column's height
		// changes without a scroll event; this is what notices.
		const observer = new ResizeObserver(pin);
		observer.observe(el);
		if (el.firstElementChild) observer.observe(el.firstElementChild);
		pin();
		return () => {
			el.removeEventListener("scroll", measure);
			observer.disconnect();
		};
		// The scroll listener above is enough while the events are the same.
	}, [empty]);

	// Hooks before the empty-state return, so their order never changes.
	const arrived = useMemo(() => withStreaming(written, streaming), [written, streaming]);
	const hidden = useCadence(personaId, arrived);
	const { activity, sleeping } = useSleep(personaId, useLanding(personaId, useSteady(live || hidden.size > 0 ? activityOf(events, streaming, hidden.size > 0) : null)));

	if (empty) {
		return (
			<div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 pb-16">
				<Avatar id={personaId} name={name} size={48} hash={avatarHash} />
				<p className="text-lg font-semibold">{name}</p>
				<p className="text-center text-sm text-ink-3">Nothing said yet. Say hello below.</p>
			</div>
		);
	}

	// A thread is said once: as its answer when one came back, else as its
	// marker. Your own answer to a card is on the card, so it draws no row.
	// A job that fired again and again with nothing drawn between is one row,
	// so this is decided on what is left to draw.
	const blocks = groupScheduled(
		arrived.filter(
			(block) =>
				!(
					block.kind === "event" &&
					(hidden.has(block.event.id) ||
						answered.has(block.event.id) ||
						reacted.lines.has(block.event.id) ||
						(block.event.kind === "delivery" && block.event.cause.kind === "answer"))
				),
		),
	);
	// Which side each block speaks from, with the machinery between two
	// messages transparent, so two agent lines around a tool call are still
	// one run of speech.
	const sides = blocks.map(sideOf);
	const sideBefore = (index: number): Side => {
		for (let at = index - 1; at >= 0; at--) if (sides[at] !== null) return sides[at]!;
		return null;
	};
	const sideAfter = (index: number): Side => {
		for (let at = index + 1; at < sides.length; at++) if (sides[at] !== null) return sides[at]!;
		return null;
	};

	return (
		<div className="relative flex min-h-0 flex-1 flex-col">
		<Scroll scrollerRef={scroller}>
			{/* `justify-end` rests a short conversation on the composer rather
			    than stranding it at the top of an empty pane. The room at the
			    bottom is the mark's for as long as the mark is there — through
			    the landing and the sleep, not just the turn — so the last bubble
			    never slides under it. It opens before the mark rises into it
			    (`wake` waits out this 200ms) and eases back once the mark is gone. */}
			<div
				className={`mx-auto flex min-h-full w-full max-w-[46rem] flex-col justify-end px-6 pt-6 transition-[padding] duration-200 ${activity !== null ? "pb-14" : "pb-6"}`}
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
							) : block.kind === "steps" ? (
								<Steps
									id={block.id}
									items={block.items}
									live={live && index === blocks.length - 1}
									shown={workShown}
									{...(onOpenWork !== undefined ? { onOpenWork, open: workOpen === block.id } : {})}
								/>
							) : (
								<Row
									personaId={personaId}
									{...(sideId !== undefined ? { sideId } : {})}
									ownerName={name}
									event={block.event}
									quote={block.event.kind === "user" && block.event.replyTo !== undefined ? said.get(block.event.replyTo) : undefined}
									top={run.top}
									bottom={run.bottom}
									speakers={speakers}
									{...(onRetryMessage && !speakers && block.event.kind === "notice" ? { onRetry: retryForNotice(events, block.event.id, onRetryMessage) } : {})}
									reactions={reacted.on.get(block.event.id)}
									{...(onReply !== undefined ? { onReply } : {})}
									{...(onReact !== undefined ? { onReact } : {})}
									{...(onOpenThread !== undefined ? { onOpenThread } : {})}
									{...(onOpenSubagent !== undefined ? { onOpenSubagent } : {})}
									{...(onOpenSide !== undefined ? { onOpenSide } : {})}
									{...(onOpenScreen !== undefined ? { onOpenScreen } : {})}
									onJump={onJump}
								/>
							)}
						</div>
					);
				})}
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
				className="control btn send jump-latest absolute bottom-3 right-8"
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

/**
 * Each kind of work stays on the mark for at least DWELL_MS before the next
 * replaces it, so an agent alternating reads and searches a few times a
 * second shows each as a pose and not as a twitch. Waiting on you and
 * writing take over at once: they are the two you need to see the moment
 * they happen.
 */
const DWELL_MS = 900;

function useSteady(activity: Activity | null): Activity | null {
	const shown = useRef<{ activity: Activity | null; at: number }>({ activity, at: Date.now() });
	const [, wake] = useReducer((n: number) => n + 1, 0);
	const now = Date.now();
	const was = shown.current;
	const work = (one: Activity | null) => one !== null && one.phase !== "blocked" && one.phase !== "writing";
	const changing = activity?.phase !== was.activity?.phase;
	const hold = changing && work(activity) && work(was.activity) && now - was.at < DWELL_MS;
	if (changing && !hold) shown.current = { activity, at: now };
	const dueIn = hold ? DWELL_MS - (now - was.at) : null;
	useEffect(() => {
		if (dueIn === null) return;
		const timer = window.setTimeout(wake, dueIn);
		return () => window.clearTimeout(timer);
	}, [dueIn]);
	return hold ? was.activity : activity;
}

/**
 * The mark wakes and goes back to sleep behind the composer: it rises from
 * behind it when a turn starts (the CSS does that on mount) and, once the
 * turn and any landing are over, it holds RESTING for SLEEP_MS — the handset
 * settles on the cradle, then the toad sinks back down out of sight. A turn
 * that starts again while it is sinking takes the mark back up. Worked out
 * during render, like the landing, so there is no frame without a mark.
 * SLEEP_MS is the settle and the sink in index.css's `.ambient-mark[data-sleeping]`.
 */
const SLEEP_MS = 800;

function useSleep(personaId: string, activity: Activity | null): { activity: Activity | null; sleeping: boolean } {
	const awake = useRef(false);
	const [sleeping, setSleeping] = useState(false);
	const off = activity === null;
	const going = off && (sleeping || awake.current);
	useEffect(() => {
		awake.current = false;
		setSleeping(false);
	}, [personaId]);
	useEffect(() => {
		if (!off) {
			awake.current = true;
			setSleeping(false);
			return;
		}
		if (!awake.current) return;
		awake.current = false;
		setSleeping(true);
		const timer = window.setTimeout(() => setSleeping(false), SLEEP_MS);
		return () => window.clearTimeout(timer);
	}, [off]);
	return activity !== null ? { activity, sleeping: false } : going ? { activity: RESTING, sleeping: true } : { activity: null, sleeping: false };
}

/**
 * The mark hangs up after the reply lands. A turn that ended while it was
 * writing keeps the mark for LANDED_MS in the `landed` phase, so the voice
 * line can reel back into the handset and drop onto the cradle; any other
 * ending — a cancel mid-tool, a refusal — lets it go at once, because
 * nothing was said to hang up on. Worked out during render rather than after
 * it, so the mark is never gone for a frame between writing and landing.
 */
function useLanding(personaId: string, activity: Activity | null): Activity | null {
	const last = useRef<ActivityPhase | null>(null);
	const [landing, setLanding] = useState(false);
	const phase = activity?.phase ?? null;
	const ended = phase === null && (landing || last.current === "writing");
	useEffect(() => {
		last.current = null;
		setLanding(false);
	}, [personaId]);
	useEffect(() => {
		if (phase !== null) {
			last.current = phase;
			setLanding(false);
			return;
		}
		if (last.current !== "writing") return;
		last.current = null;
		setLanding(true);
		const timer = window.setTimeout(() => setLanding(false), LANDED_MS);
		return () => window.clearTimeout(timer);
	}, [phase]);
	return activity ?? (ended ? LANDED : null);
}

/**
 * The agent's bubbles land to a beat. The ids still waiting their turn, and a
 * re-render when the next is due. Everything else on the tape shows at once.
 */
function useCadence(personaId: string, blocks: Block[]): ReadonlySet<string> {
	const cadence = useRef<Cadence>(REST);
	const [, wake] = useReducer((n: number) => n + 1, 0);
	useEffect(() => {
		cadence.current = REST;
	}, [personaId]);
	const bubbles: Bubble[] = [];
	for (const block of blocks) {
		if (block.kind === "event" && block.event.kind === "agent") {
			bubbles.push({ id: block.event.id, text: block.event.text, ts: block.event.ts });
		}
	}
	const next = step(cadence.current, bubbles, Date.now());
	cadence.current = next.cadence;
	const dueIn = next.dueIn;
	useEffect(() => {
		if (dueIn === null) return;
		const timer = window.setTimeout(wake, dueIn);
		return () => window.clearTimeout(timer);
	}, [dueIn, next.hidden.length]);
	return new Set(next.hidden);
}

/**
 * Whose bubble a block is. Anything else on screen — a card, a notice, a
 * chapter line — is `other`, and breaks a run; the machinery between two
 * messages and a turn that draws nothing are `null`, and do not.
 */
type Side = "me" | "them" | "other" | null;

/** Whether the bubble continues a run from the same side above, and below. */
type Run = { top: boolean; bottom: boolean };

function sideOf(block: Block): Side {
	if (block.kind === "scheduled") return "other";
	if (block.kind !== "event") return null;
	const event = block.event;
	if (event.kind === "user") return event.scheduled === undefined ? "me" : "other";
	if (event.kind === "agent") return "them";
	if (event.kind === "turn") return event.stopReason === "end_turn" ? null : "other";
	return "other";
}

function runClass(run: Run): string {
	return `${run.top ? "said-run-top" : ""} ${run.bottom ? "said-run-bottom" : ""}`;
}

/**
 * Fold runs of thoughts and tools into one block; a streaming thought joins
 * the tail. A reply being written shows each bubble once it is whole, never a
 * bubble that will still grow, cut the way the desk will write it and under
 * the ids it will use, so the written lines replace them in place.
 */
/** The written events as rows: runs of thoughts and tool calls fold into one block of steps. */
function toBlocks(events: TranscriptEvent[]): Block[] {
	const blocks: Block[] = [];
	for (const event of events) {
		if (event.kind === "computer_pull") continue;
		if (event.kind === "thought" || event.kind === "tool") {
			const tail = blocks[blocks.length - 1];
			if (tail?.kind === "steps") tail.items.push(event);
			else blocks.push({ kind: "steps", id: event.id, ts: event.ts, items: [event] });
		} else {
			blocks.push({ kind: "event", event });
		}
	}
	return blocks;
}

/**
 * The written rows, then what is streaming after them. The written blocks are
 * shared with the last render, so their rows are skipped; a streamed thought
 * joining the last block of steps gets a copy of that block, never an edit.
 */
function withStreaming(written: Block[], streaming: Streaming[]): Block[] {
	if (streaming.length === 0) return written;
	const blocks = written.slice();
	for (const one of streaming) {
		if (one.kind === "agent") {
			const base = one.bubbleOf?.base ?? one.messageId;
			const start = one.bubbleOf?.index ?? 0;
			pacedLive(wholeBubbles(one.text)).forEach((text, index) => {
				blocks.push({
					kind: "event",
					event: { kind: "agent", id: bubbleId(base, start + index), ts: Date.now(), text },
				});
			});
			continue;
		}
		const thought: Step = { kind: "thought", id: one.messageId, ts: Date.now(), text: one.text };
		const tail = blocks[blocks.length - 1];
		if (tail?.kind === "steps") blocks[blocks.length - 1] = { ...tail, items: [...tail.items, thought] };
		else blocks.push({ kind: "steps", id: thought.id, ts: thought.ts, items: [thought] });
	}
	return blocks;
}

function block_ts(block: Block): number {
	return block.kind === "event" ? block.event.ts : block.ts;
}

function isChapter(block: Block): boolean {
	return block.kind === "event" && block.event.kind === "chapter";
}

/**
 * A search hit or a reply's quote: unpin, bring the row to the middle, and
 * light it briefly. The tape arrives after the jump is asked for when
 * Everywhere opens another teammate, so this waits until the fold contains
 * the id. `found` going true is the retry; a later append does not change
 * `found` and so does not jump.
 */
function useScrollToEvent(
	scroller: RefObject<HTMLDivElement | null>,
	pinned: RefObject<boolean>,
	focus: { eventId: string; at: number } | null,
	events: TranscriptEvent[],
): void {
	const found = focus !== null && events.some((one) => one.id === focus.eventId);
	useEffect(() => {
		if (!focus || !found) return;
		const root = scroller.current;
		// A step lives inside its block, so the block is what is found.
		const row =
			root?.querySelector<HTMLElement>(`[data-event-id="${CSS.escape(focus.eventId)}"]`) ??
			root?.querySelector<HTMLElement>(`[data-step-id="${CSS.escape(focus.eventId)}"]`)?.closest<HTMLElement>("[data-event-id]");
		if (!root || !row) return;
		pinned.current = false;
		const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
		row.scrollIntoView({ block: "center", behavior: reduce ? "auto" : "smooth" });
		row.classList.add("row-lit");
		const timer = window.setTimeout(() => row.classList.remove("row-lit"), 1_800);
		return () => window.clearTimeout(timer);
	}, [focus, found, scroller, pinned]);
}

/** Only an operator message from this turn can be offered again. A schedule,
 * peer delivery, or a later message is not a failed draft to replay. */
export function retryForNotice(events: TranscriptEvent[], noticeId: string,
    refill: (message: Extract<TranscriptEvent, { kind: "user" }>) => void): (() => void) | undefined {
    const index = events.findIndex((event) => event.id === noticeId);
    if (index < 0 || events.slice(index + 1).some((event) => ["user", "delivery", "chapter"].includes(event.kind))) return;
    for (let at = index - 1; at >= 0; at--) {
        const event = events[at]!;
        if (event.kind === "user") return event.scheduled === undefined ? () => refill(event) : undefined;
        if (["delivery", "chapter", "turn"].includes(event.kind)) return;
    }
}

/**
 * One written row. Memoized: while a reply streams, every row above it gets
 * the same props and is skipped, so the cost of a word is the live bubble's.
 */
const Row = memo(function Row({
	personaId,
	sideId,
	ownerName,
	event,
	quote,
	top,
	bottom,
	speakers,
	reactions,
	onReply,
	onReact,
	onRetry,
	onOpenThread,
	onOpenSubagent,
	onOpenSide,
	onOpenScreen,
	onJump,
}: {
	personaId: string;
	sideId?: string;
	/** Whose tape this is: the teammate, so a card can speak of it in the third person. */
	ownerName: string;
	event: Exclude<TranscriptEvent, Step>;
	/** What the message this one replies to said, when it is on the tape. */
	quote: string | undefined;
	top: boolean;
	bottom: boolean;
	speakers: Speakers | undefined;
	/** Emoji the phone or this window sent as lines of their own, folded onto this one. */
	reactions: string[] | undefined;
	onRetry?: (() => void) | undefined;
	onReply?(target: ReplyTarget): void;
	onReact?(target: ReactTarget, emoji: string): void;
	onOpenThread?(thread: ThreadRef): void;
	onOpenSubagent?(event: SubagentEvent): void;
	/** Opens a side thread in the right-hand pane from its line. */
	onOpenSide?(event: SideEvent): void;
	onOpenScreen?(): void;
	onJump(eventId: string): void;
}) {
	const run: Run = { top, bottom };
	const own = event.kind === "user" || event.kind === "agent" ? event.reactions : undefined;
	const worn = reactions === undefined ? own : [...(own ?? []), ...reactions];
	switch (event.kind) {
		case "user":
			return event.scheduled !== undefined ? (
				<ScheduledLine name={event.scheduled.name} prompt={event.text} />
			) : speakers !== undefined ? (
				<NamedSay name={speakers.mine === "user" ? speakers.me : speakers.them} mine={speakers.mine === "user"} text={event.text} />
			) : (
				<UserBubble event={event} quote={quote} run={run} reactions={worn} onJump={onJump} {...(onReply !== undefined ? { onReply } : {})} />
			);

		case "agent":
			return speakers !== undefined ? (
				<NamedSay
					name={speakers.mine === "agent" ? speakers.me : speakers.them}
					mine={speakers.mine === "agent"}
					text={event.text}
				/>
			) : (
				<AgentSay
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
			if (event.level === "error") return <ErrorCard text={event.text} {...(!speakers ? { personaId } : {})} {...(onRetry ? { onRetry } : {})} />;
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
			return <Permission personaId={personaId} {...(sideId !== undefined ? { sideId } : {})} event={event} />;

		case "plan":
			return <Plan entries={event.entries} />;

		case "human_action":
			return <HumanAction personaId={personaId} event={event} {...(onOpenScreen !== undefined ? { onOpenScreen } : {})} />;

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
				<button type="button" className="hung-line" data-missed={event.status === "failed" || undefined} onClick={() => onOpenThread?.(event)}>
					<Avatar id={event.withPersonaId} name={event.withName} size={16} />
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
			return (
				<button
					type="button"
					className="hung-line"
					data-missed={deliveryMissed(event) || undefined}
					onClick={() => onOpenThread?.({
						threadKey: cause.threadKey,
						withName: cause.name,
						...(cause.kind === "handoff" ? { handoff: cause } : {}),
					})}
				>
					<Avatar id={cause.personaId} name={line.name} size={16} />
					<span className="min-w-0 truncate">
						<span className="hung-line-name">{line.name}</span> {line.said}
					</span>
					<ChevronRightIcon />
				</button>
			);
		}

		/* Work the teammate handed to a subagent: one quiet line that fills
		 * in as the run goes, the way a peer thread is one. Pressing it opens
		 * the run in the work card; while it runs the band names it too. */
		case "subagent":
			return (
				<button
					type="button"
					className="rule-line rule-line-plain w-full"
					style={event.status === "failed" ? { color: "var(--warn)" } : undefined}
					onClick={() => onOpenSubagent?.(event)}
				>
					<span className="min-w-0 truncate">{`Subagent · ${event.title}`}</span>
					<span className="shrink-0">{`· ${subagentState(event)}`}</span>
				</button>
			);

		/* A side thread the person started beside this conversation: one quiet
		 * line, "Started a side thread" while it runs, a title and a one-line
		 * result once it is archived. Either way it opens the thread in the
		 * work card. */
		case "side": {
			const line = sideLine(event);
			return (
				<button
					type="button"
					className="rule-line rule-line-plain w-full"
					aria-label={`${line.text}. Open the side thread`}
					onClick={() => onOpenSide?.(event)}
				>
					<span className="min-w-0 truncate">{line.text}</span>
					<span className="shrink-0">· Open</span>
				</button>
			);
		}

		case "computer_frame":
			return <ComputerFrame dataUrl={event.dataUrl} />;

		/* Older tapes carry a download's progress; it now lives on the
		 * computer's button, and the conversation never shows it. */
		case "computer_pull":
			return null;
	}
});

export type SubagentEvent = Extract<TranscriptEvent, { kind: "subagent" }>;
export type SideEvent = Extract<TranscriptEvent, { kind: "side" }>;

/**
 * What a side thread's line in the conversation says: while it runs, that it
 * started; parked, that it is waiting to be picked up; once archived, its
 * title and what came of it in one line, and how it ended when nobody said it
 * was done.
 */
export function sideLine(event: SideEvent): { text: string } {
	if (event.status === "live") return { text: `Started a side thread · ${event.title}` };
	if (event.status === "parked") return { text: `Side thread · ${event.title} · parked` };
	const ending = event.archivedBy === "stopped" ? "stopped" : event.archivedBy === "idle" ? "archived, idle" : "";
	const outcome = event.result !== undefined && event.result !== "" ? event.result : "archived";
	return { text: `Side thread · ${event.title} · ${outcome}${ending !== "" && event.result !== undefined ? ` · ${ending}` : ""}` };
}

/** What opens a thread: a peer marker is one, and a delivery names one. */
export type ThreadRef = {
	threadKey: string;
	withName: string;
	handoff?: Extract<DeliveryCause, { kind: "handoff" }>;
};

type DeliveryEvent = Extract<TranscriptEvent, { kind: "delivery" }>;

/**
 * A colleague's delivery, in the colleague's voice: who, and the start of
 * what they said. The answer is what happened, so the line quotes it rather
 * than the question it answers. `null` for your own answer to a card: the
 * card already says it.
 */
export function deliveryLine(event: DeliveryEvent): { name: string; said: string } | null {
	const cause = event.cause;
	if (cause.kind === "answer") return null;
	if (cause.kind === "peer" && cause.status === "failed")
		return { name: cause.name, said: cause.about ? `didn't answer · ${cause.about}` : "didn't answer" };
	const said = plain(firstLine(event.text)) || cause.about;
	if (cause.kind !== "handoff") return { name: cause.name, said };
	// A handoff still behind the teammate's current turn says so; once taken up it is just what was handed.
	return { name: cause.name, said: `handed you: ${said}${event.receipt === "sent" ? " · queued" : ""}` };
}

/** A quoted line reads as words, not as the markdown it was written in. */
function plain(line: string): string {
	return line.replace(/[`*_]+/g, "");
}

type PeerEvent = Extract<TranscriptEvent, { kind: "peer" }>;

/**
 * The marker's line, in plain words: who, whether it is still going, and how
 * much was said. Once its answer is delivered the marker is dropped (see
 * `superseded`), so this mostly reads while a reply is awaited, or on the
 * side that was asked.
 */
export function peerLine(event: PeerEvent): string {
	const who = event.seat === "client" ? `${event.withName} (outside the room)` : event.withName;
	const what =
		event.status === "failed"
			? `${who} didn't answer`
			: event.role === "caller"
				? event.status === "done"
					? `Talked with ${who}`
					: `Waiting on ${who}`
				: `${who} asked`;
	return event.exchanges > 1 ? `${what} · ${event.exchanges} messages` : what;
}

/** Markers a later delivery already stands for, in the same conversation. */
export function superseded(events: readonly TranscriptEvent[]): Set<string> {
	const answered = new Set<string>();
	const hidden = new Set<string>();
	for (let at = events.length - 1; at >= 0; at--) {
		const event = events[at]!;
		if (event.kind === "delivery" && event.cause.kind !== "answer") answered.add(event.cause.threadKey);
		else if (event.kind === "peer" && event.status !== "waiting" && answered.has(event.threadKey)) hidden.add(event.id);
	}
	return hidden;
}

/** Whether a delivery says something did not come back. */
export function deliveryMissed(event: DeliveryEvent): boolean {
	return event.cause.kind === "peer" && event.cause.status === "failed";
}

/**
 * Why a running turn began, when the last thing before it was a delivery
 * rather than a word from the person: answering a colleague or picking up
 * an answer that arrived while the teammate was away. A person's message
 * or a completed turn ends that cause, so a later turn cannot inherit it.
 */
export function turnCauseLine(events: TranscriptEvent[]): string | null {
	for (let index = events.length - 1; index >= 0; index--) {
		const event = events[index]!;
		if (event.kind === "user" || event.kind === "turn") return null;
		if (event.kind === "delivery") {
			// A queued delivery must not rename the person's current turn.
			if (event.receipt !== "read") continue;
			const cause = event.cause;
			return cause.kind === "answer" ? "Picking up your answer" : `Answering ${cause.name}`;
		}
	}
	return null;
}

/** Where a subagent's run has got to, in the words its line ends with. */
export function subagentState(event: SubagentEvent): string {
	const took = event.elapsedMs === undefined ? "" : ` after ${runWords(event.elapsedMs)}`;
	switch (event.status) {
		case "running":
			return "working";
		case "done":
			return event.elapsedMs === undefined ? "done" : `done in ${runWords(event.elapsedMs)}`;
		case "failed":
			return `failed${took}`;
		case "cancelled":
			return `stopped${took}`;
	}
}

/** A run is seconds to many minutes long: say it the way a person would. */
function runWords(ms: number): string {
	const seconds = Math.round(ms / 1000);
	if (seconds < 1) return "under a second";
	if (seconds < 60) return `${seconds} s`;
	const minutes = Math.floor(seconds / 60);
	const rest = seconds % 60;
	return rest === 0 ? `${minutes} min` : `${minutes} min ${rest} s`;
}

/**
 * The machinery between two messages, as one caption: a count, closed
 * until you open it. While the agent is still on it there is no caption at
 * all — the typing bubble is the caption. In a conversation the caption
 * opens the work in the pane beside it (`onOpen`); in a thread or a run it
 * opens in place, and the live rows show only if you pressed the bubble.
 */
const Steps = memo(function Steps({
	id,
	items,
	live,
	shown,
	onOpenWork,
	open: paneOpen,
}: {
	id: string;
	items: Step[];
	live: boolean;
	shown: boolean;
	onOpenWork?(blockId: string): void;
	open?: boolean;
}) {
	const [toggled, setToggled] = useState(false);
	const inPane = onOpenWork !== undefined;
	const open = inPane ? false : live ? shown : toggled;
	const summary = stepsSummary(items);
	const failed = items.some((one) => one.kind === "tool" && one.status === "failed");

	return (
		<div className="my-2">
			{!live && (
				<button
					type="button"
					className="steps-caption"
					aria-expanded={inPane ? paneOpen === true : open}
					onClick={() => (inPane ? onOpenWork(id) : setToggled(!open))}
				>
					<span className={`truncate ${failed ? "text-danger" : ""}`}>{summary}</span>
					{open ? <ChevronDownIcon /> : <ChevronRightIcon />}
				</button>
			)}
			{open && (
				<div className="steps ml-[11px]">
					<StepRows items={items} />
				</div>
			)}
		</div>
	);
});

/** "12 steps", "12 steps · one failed": the caption, and the work pane's line. */
export function stepsSummary(items: Step[]): string {
	const failed = items.some((one) => one.kind === "tool" && one.status === "failed");
	return `${items.length} ${items.length === 1 ? "step" : "steps"}${failed ? " · one failed" : ""}`;
}

/**
 * Each step as a row, its detail behind a press: a run of thoughts as one
 * "Thinking" row, since harnesses send thinking in pieces that break
 * mid-sentence, and a tool with its output.
 */
export function StepRows({ items, settled = false }: { items: Step[]; /** The turn is over: nothing in it is still running. */ settled?: boolean }) {
	const rows: ({ kind: "thinking"; id: string; pieces: string[] } | Extract<Step, { kind: "tool" }>)[] = [];
	for (const item of items) {
		const last = rows[rows.length - 1];
		if (item.kind === "thought") {
			if (last?.kind === "thinking") last.pieces.push(item.text);
			else rows.push({ kind: "thinking", id: item.id, pieces: [item.text] });
		} else rows.push(item);
	}
	return rows.map((row, at) =>
		row.kind === "thinking" ? (
			<Thought key={row.id} id={row.id} text={joinThoughts(row.pieces)} live={!settled && at === rows.length - 1} />
		) : (
			<Tool
				key={row.id}
				id={row.id}
				title={row.title}
				// A harness that never closed a call leaves it running on the tape after the turn ended.
				status={settled && (row.status === "in_progress" || row.status === "pending") ? "completed" : row.status}
				output={row.output}
			/>
		),
	);
}

/**
 * The runs of steps in a tape, in order, each under the id its caption
 * opens it by. What is streaming joins the last run, as it does on screen.
 */
export function stepRuns(events: TranscriptEvent[], streaming: Streaming[]): { id: string; items: Step[] }[] {
	return withStreaming(toBlocks(events), streaming).flatMap((block) => (block.kind === "steps" ? [{ id: block.id, items: block.items }] : []));
}

/**
 * How a run of steps ended: still going, or stopped short by a turn that
 * did not end in its own time. Read from the first turn line after it.
 */
export function runEnding(events: TranscriptEvent[], items: Step[]): "done" | "stopped" {
	const last = items[items.length - 1];
	if (last === undefined) return "done";
	const at = events.findIndex((event) => event.id === last.id);
	if (at < 0) return "done";
	for (const event of events.slice(at + 1)) {
		if (event.kind === "turn") return event.stopReason === "end_turn" ? "done" : "stopped";
	}
	return "done";
}

export type { Step };

/**
 * An agent's line, focusable so R can answer it without a pointer. A file
 * it sent sits under its words, or is the whole bubble when it came alone.
 */
function AgentSay({
	personaId,
	event,
	run,
	reactions,
	onReply,
	onReact,
}: {
	personaId: string;
	event: Extract<TranscriptEvent, { kind: "agent" }>;
	run: Run;
	reactions: string[] | undefined;
	onReply?(target: ReplyTarget): void;
	onReact?(target: ReactTarget, emoji: string): void;
}) {
	const reply = () => onReply?.({ eventId: event.id, text: lineOf(event) });
	const actions: BubbleActionsProps = {
		copy: () => void writeClipboard(event.text.trim() !== "" ? event.text : lineOf(event)),
		...(onReply !== undefined ? { reply } : {}),
		...(onReact !== undefined ? { react: (emoji: string) => onReact({ eventId: event.id, text: event.text || lineOf(event) }, emoji) } : {}),
	};
	return (
		<div className={`said-group relative ${run.top ? "mt-1" : "mt-3"}`}>
			<div
				className={`speech said-them ${runClass(run)}`}
				tabIndex={onReply === undefined ? undefined : 0}
				onContextMenu={(click) => bubbleMenu(click, actions)}
				onKeyDown={(key) => {
					if (onReply === undefined) return;
					if (key.repeat) return;
					if (key.ctrlKey || key.altKey || key.metaKey) return;
					if (key.key !== "r" && key.key !== "R") return;
					key.preventDefault();
					reply();
				}}
			>
				{event.text.trim() !== "" && <Markdown text={event.text} />}
				{event.attachments?.map((file, index) => (
					<SentFile key={file.path} personaId={personaId} eventId={event.id} index={index} file={file} />
				))}
				<Reactions emoji={reactions} />
				<BubbleActions {...actions} />
			</div>
		</div>
	);
}

/**
 * What you typed, and — when this line answers another — that other line
 * quoted from the fold. A leading `>` block is stripped only then: imported
 * tapes still carry the quote the old composer wrote into the text, and the
 * bar above already shows the original. Files that rode with the line sit
 * under the words, named from the event, with the path on hover.
 */
/** A named line in a peer thread: this teammate on the right, the other on the left. */
function NamedSay({ name, mine, text }: { name: string; mine: boolean; text: string }) {
	if (mine) {
		return (
			<div className="mt-3 flex flex-col items-end">
				<p className="said-name">{name}</p>
				<div className="speech said-me">{text}</div>
			</div>
		);
	}
	return (
		<div className="said-group relative mt-3">
			<p className="said-name">{name}</p>
			<div className="speech said-them">
				<Markdown text={text} />
			</div>
		</div>
	);
}

function UserBubble({
	event,
	quote,
	run,
	reactions,
	onJump,
	onReply,
}: {
	event: Extract<TranscriptEvent, { kind: "user" }>;
	quote: string | undefined;
	run: Run;
	reactions: string[] | undefined;
	onJump(eventId: string): void;
	onReply?(target: ReplyTarget): void;
}) {
	const answered = event.replyTo;
	const text = quote !== undefined ? unquoted(event.text) : event.text;
	// A line still on its way has no id the desk knows, so it cannot be answered yet.
	const sent = !event.id.startsWith("saying:");
	const actions: BubbleActionsProps = {
		copy: () => void writeClipboard(text),
		...(onReply !== undefined && sent ? { reply: () => onReply({ eventId: event.id, text: lineOf({ text }) }) } : {}),
	};
	return (
		<div className={`said-group flex justify-end ${run.top ? "mt-1" : "mt-3"}`}>
			<div className={`speech said-me ${runClass(run)}`} onContextMenu={(click) => bubbleMenu(click, actions)}>
				{quote !== undefined && answered !== undefined && (
					<button type="button" className="quote" title="Go to the message" onClick={() => onJump(answered)}>
						{quote}
					</button>
				)}
				{text}
				{event.attachments !== undefined && event.attachments.length > 0 && (
					<ul className="mt-2 flex flex-wrap gap-1" style={{ whiteSpace: "normal" }}>
						{event.attachments.map((item) => (
							<li key={item.path} className="chip max-w-full pr-2" title={item.path}>
								<span className="chip-name">{item.name}</span>
							</li>
						))}
					</ul>
				)}
				{event.receipt !== undefined && <Ticks read={event.receipt === "read"} />}
				<Reactions emoji={reactions} />
				<BubbleActions {...actions} />
			</div>
		</div>
	);
}

/**
 * How far the line got: one tick once it is on the tape, two once the agent
 * has it in context. Nothing un-reads, so the ticks only ever climb.
 */
function Ticks({ read }: { read: boolean }) {
	return (
		<span className={`ticks ${read ? "ticks-read" : ""}`} role="img" aria-label={read ? "Read" : "Sent"} title={read ? "Read" : "Sent"}>
			<CheckIcon />
			{read && <CheckIcon />}
		</span>
	);
}

/** The six the phone's long-press offers, in its order, so both ends react alike. */
export const REACTIONS = ["👍", "❤️", "😂", "🔥", "👀", "🙏"] as const;

type BubbleActionsProps = { copy(): void; reply?(): void; react?(emoji: string): void };

/**
 * What a bubble offers a pointer: react, reply, copy. It sits beside the
 * bubble on its open side and shows on hover or focus, the desktop's long
 * press. The face swaps the row for the six emoji until the pointer leaves.
 */
function BubbleActions({ copy, reply, react }: BubbleActionsProps) {
	const [picking, setPicking] = useState(false);
	const [copied, setCopied] = useState(false);
	useEffect(() => {
		if (!copied) return;
		const done = setTimeout(() => setCopied(false), 1200);
		return () => clearTimeout(done);
	}, [copied]);
	return (
		<span className="bubble-actions" role="toolbar" aria-label="Message actions" onMouseLeave={() => setPicking(false)}>
			{picking && react !== undefined ? (
				REACTIONS.map((emoji) => (
					<button
						key={emoji}
						type="button"
						className="bubble-action bubble-emoji"
						tabIndex={-1}
						aria-label={`React ${emoji}`}
						onClick={() => {
							setPicking(false);
							react(emoji);
						}}
					>
						{emoji}
					</button>
				))
			) : (
				<>
					{react !== undefined && (
						<button type="button" className="bubble-action" tabIndex={-1} title="React" aria-label="React" onClick={() => setPicking(true)}>
							<SmileIcon />
						</button>
					)}
					{reply !== undefined && (
						<button type="button" className="bubble-action" tabIndex={-1} title={`Reply (${chordKeys("reply")})`} aria-label="Reply" onClick={reply}>
							<ReplyIcon />
						</button>
					)}
					<button
						type="button"
						className="bubble-action"
						tabIndex={-1}
						title={copied ? "Copied" : "Copy"}
						aria-label={copied ? "Copied" : "Copy"}
						onClick={() => {
							copy();
							setCopied(true);
						}}
					>
						{copied ? <CheckIcon /> : <CopyIcon />}
					</button>
				</>
			)}
		</span>
	);
}

/**
 * A right-click on a bubble opens its menu, unless words in it are selected:
 * then it is the platform's own menu, so a few words can still be copied.
 */
function bubbleMenu(click: MouseEvent<HTMLElement>, actions: BubbleActionsProps) {
	const selection = window.getSelection();
	if (selection !== null && !selection.isCollapsed && click.currentTarget.contains(selection.anchorNode)) return;
	click.preventDefault();
	void popupMessageMenu({
		reactions: REACTIONS,
		onCopy: actions.copy,
		...(actions.reply !== undefined ? { onReply: actions.reply } : {}),
		...(actions.react !== undefined ? { onReact: actions.react } : {}),
	});
}

/** One line quoted the way the phone quotes it: whitespace folded, 140 characters. */
export function reactionQuote(text: string): string {
	return `> ${text
		.split("\n")
		.map((line) => line.trim())
		.filter(Boolean)
		.join(" ")
		.slice(0, 140)}`;
}

const EMOJI_ONLY = /^(?:\p{Extended_Pictographic}|\p{Emoji_Modifier}|\u200d|\ufe0f)+$/u;

/**
 * Reactions sent as lines of their own — a quoted line and nothing but an
 * emoji, the phone's shape and this window's — folded onto the line they
 * answer, found by its id or else by the quote. The same rule as the phone's
 * `conversationItems`, so both ends draw one tape alike.
 */
export function foldReactions(events: TranscriptEvent[]): { lines: Set<string>; on: Map<string, string[]> } {
	const lines = new Set<string>();
	const on = new Map<string, string[]>();
	events.forEach((event, index) => {
		if (event.kind !== "user" || (event.attachments?.length ?? 0) > 0) return;
		const match = /^(> [^\n]*)\n+([^\n]+)$/.exec(event.text.trim());
		if (!match || !EMOJI_ONLY.test(match[2]!.trim())) return;
		let target = event.replyTo;
		if (target === undefined) {
			for (let i = index - 1; i >= 0; i--) {
				const candidate = events[i]!;
				if (candidate.kind !== "user" && candidate.kind !== "agent") continue;
				const said = candidate.kind === "agent" ? candidate.text || candidate.attachments?.[0]?.name || "" : candidate.text;
				if (reactionQuote(said) === match[1]) {
					target = candidate.id;
					break;
				}
			}
		}
		if (target === undefined) return;
		lines.add(event.id);
		on.set(target, [...(on.get(target) ?? []), match[2]!.trim()]);
	});
	return { lines, on };
}

/** What the other side said with an emoji, tucked under the bubble's corner. */
function Reactions({ emoji }: { emoji: string[] | undefined }) {
	if (emoji === undefined || emoji.length === 0) return null;
	return (
		<span className="reactions" aria-label={`Reactions: ${emoji.join(" ")}`}>
			{emoji.map((one, index) => (
				<span key={`${one}-${index}`}>{one}</span>
			))}
		</span>
	);
}

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

/** First line of a say, or nothing — a missing or empty original is not a quote. */
function quotedLine(event: TranscriptEvent): string | undefined {
	if (event.kind !== "user" && event.kind !== "agent") return undefined;
	const line = lineOf(event);
	return line.length > 0 ? line : undefined;
}

/** A say's first line, or the names of its files when it came without words. */
function lineOf(event: { text: string; attachments?: Attachment[] }): string {
	const line = firstLine(event.text);
	if (line.length > 0) return line;
	return firstLine((event.attachments ?? []).map((file) => file.name).join(", "));
}

/** The stored text minus a leading quote block the old composer prepended. */
function unquoted(text: string): string {
	const lines = text.split("\n");
	let end = 0;
	while (end < lines.length && lines[end]!.startsWith(">")) end++;
	if (end === 0) return text;
	return lines.slice(end).join("\n").replace(/^\n+/, "");
}

/** What the agent was thinking: "Thinking" until pressed, the whole thought after. */
function Thought({ id, text, live }: { id: string; text: string; /** Still arriving. */ live: boolean }) {
	const [open, setOpen] = useState(false);
	return (
		<div data-step-id={id}>
			<button type="button" className="step" aria-expanded={open} onClick={() => setOpen((was) => !was)}>
				<span aria-hidden="true" className={`step-mark ${live ? "beat" : ""}`} style={{ boxShadow: "inset 0 0 0 1.5px var(--ink-4)" }} />
				<span className="step-title font-sans italic text-ink-3">Thinking</span>
				{open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />}
			</button>
			{open && <div className="step-out font-sans not-italic">{text}</div>}
		</div>
	);
}

const STATUS_INK: Record<ToolStatus, string> = {
	pending: "var(--ink-4)",
	in_progress: "var(--accent)",
	completed: "var(--ink-4)",
	failed: "var(--danger)",
};

/** A tool call: what it was, how it went, and its output behind a press. */
function Tool({
	id,
	title,
	status,
	output,
}: {
	id: string;
	title: string;
	status: ToolStatus;
	output: ToolOutput[] | undefined;
}) {
	const [open, setOpen] = useState(false);
	// Some harnesses record an empty text output; that is nothing to open.
	const hasOutput = output !== undefined && output.some((one) => one.type === "diff" || one.text.trim() !== "");
	return (
		<div data-step-id={id}>
			<button
				type="button"
				className="step"
				aria-expanded={hasOutput ? open : undefined}
				disabled={!hasOutput}
				onClick={() => setOpen((was) => !was)}
			>
				<span
					aria-hidden="true"
					className={`step-mark ${status === "in_progress" ? "beat" : ""}`}
					style={{ background: STATUS_INK[status] }}
				/>
				<span className="step-title" title={title}>
					{stepTitle(title)}
				</span>
				{status === "failed" && <span className="shrink-0 text-danger">failed</span>}
				{status === "in_progress" && <span className="shrink-0 text-ink-3">running</span>}
				{hasOutput && (open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />)}
			</button>
			{open && hasOutput && <StepOutput output={output} />}
		</div>
	);
}


/** A tool's output, cleaned once per output rather than on every streamed word. */
const StepOutput = memo(function StepOutput({ output }: { output: ToolOutput[] }) {
	const parts = useMemo(
		() => output.map((one) => ({ edit: one.type === "diff", text: outputText(one) })).filter((part) => part.text !== ""),
		[output],
	);
	return (
		<div className="step-out">
			{parts.map((part, at) => (
				<div key={at} className="step-out-part">
					{part.edit ? <EditLines text={part.text} /> : part.text}
				</div>
			))}
		</div>
	);
});

/** An edit: its path, then its lines tinted by whether they went or came, a run of one kind as one block. */
function EditLines({ text }: { text: string }) {
	const [path, ...lines] = text.split("\n");
	const runs: { kind: string; lines: string[] }[] = [];
	for (const line of lines) {
		const kind = line.startsWith("+ ") ? "step-edit-add" : line.startsWith("- ") ? "step-edit-del" : "step-edit-same";
		const last = runs[runs.length - 1];
		if (last?.kind === kind) last.lines.push(line);
		else runs.push({ kind, lines: [line] });
	}
	return (
		<>
			<div className="step-edit-path">{path}</div>
			{runs.map((run, at) => (
				<div key={at} className={run.kind}>
					{run.lines.join("\n")}
				</div>
			))}
		</>
	);
}

/**
 * A permission the agent asked. The choice is history when the tape already
 * names it; otherwise the options are the answer, and they stay dead while
 * that answer is in flight so a second click cannot race the first.
 */
function Permission({
	personaId,
	sideId,
	event,
}: {
	personaId: string;
	sideId?: string;
	event: Extract<TranscriptEvent, { kind: "permission" }>;
}) {
	const [answering, setAnswering] = useState(false);
	const chosen = chosenOption(event);

	const answer = (optionId: string) => {
		if (answering || event.decision !== undefined) return;
		setAnswering(true);
		void (sideId !== undefined
			? wire.command("side.answer_permission", { sideId, requestId: event.requestId, optionId })
			: wire.command("session.answer_permission", { personaId, requestId: event.requestId, optionId })
		).catch(() => setAnswering(false));
	};

	return (
		<div className={`card mt-3 ${chosen === undefined ? "card-live" : ""}`}>
			<p className="eyebrow mb-1">{chosen === undefined ? "Asking permission" : "Asked permission"}</p>
			<p className="selectable" style={{ whiteSpace: "pre-wrap" }}>{event.title}</p>
			{chosen !== undefined ? (
				<p className="mt-1.5 flex items-center gap-1.5 text-sm text-ink-3">
					{event.decision !== "expired" && <CheckIcon className="text-ink-4" />}
					{chosen}
				</p>
			) : (
				<div className="card-actions">
					{event.options.map((option) => (
						<button
							key={option.optionId}
							type="button"
							disabled={answering}
							className={`control ${optionKindClass(option)}`}
							onClick={() => answer(option.optionId)}
						>
							{option.name}
						</button>
					))}
				</div>
			)}
		</div>
	);
}

function chosenOption(event: Extract<TranscriptEvent, { kind: "permission" }>): string | undefined {
	if (event.decision === "expired") return "Expired unanswered";
	if (event.decidedOptionName !== undefined && event.decidedOptionName !== "") {
		return event.decidedOptionName;
	}
	if (event.decision === undefined) return undefined;
	const option = event.options.find((one) => one.optionId === event.decision);
	return option?.name ?? event.decision;
}

function optionKindClass(option: PermissionOption): string {
	return option.kind?.startsWith("allow") ? "btn-primary" : "btn";
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
		return <CheckIcon className="mt-px shrink-0 text-accent" />;
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

const AFTERLIFE: Record<HumanActionStatus, string> = {
	pending: "Needs you",
	done: "Done",
	dismissed: "Declined",
	expired: "Expired",
};

/**
 * The agent asked for hands it does not have. A pending card is answered
 * here; a decided one is the outcome on the tape. The note goes with
 * either answer and reaches the agent word for word, so a card that asked
 * a question is answered in the same place it was asked. Enter is Done.
 */
function HumanAction({
	personaId,
	event,
	onOpenScreen,
}: {
	personaId: string;
	event: Extract<TranscriptEvent, { kind: "human_action" }>;
	onOpenScreen?(): void;
}) {
	const [answering, setAnswering] = useState(false);
	const [noting, setNoting] = useState(false);
	const [note, setNote] = useState("");

	const answer = (status: HumanAnswer) => {
		if (answering || event.status !== "pending") return;
		setAnswering(true);
		const trimmed = note.trim();
		const params = { personaId, actionId: event.actionId, status };
		void wire
			.command("human.answer", trimmed ? { ...params, note: trimmed } : params)
			.catch(() => setAnswering(false));
	};

	if (event.status !== "pending") {
		return (
			<div className="card mt-3">
				<p className="eyebrow mb-1">
					Needed you · {AFTERLIFE[event.status]}
					{event.note?.trim() ? <span className="normal-case"> · {event.note.trim()}</span> : null}
				</p>
				<p className="selectable text-ink-2">{event.reason}</p>
			</div>
		);
	}

	return (
		<div className="card card-live mt-3">
			<p className="eyebrow mb-1">Needs you</p>
			<p className="selectable">{event.reason}</p>
			{onOpenScreen !== undefined && (
				<p className="mt-1.5 text-sm text-ink-3">
					The screen is yours until you press Done. What you type there goes to the desktop, not to the teammate.
				</p>
			)}
			{noting && (
				<input
					className="field mt-2.5 w-full"
					aria-label="A note for the teammate"
					placeholder="A note for the teammate"
					autoComplete="off"
					autoFocus
					value={note}
					onChange={(change) => setNote(change.target.value)}
					onKeyDown={(key) => {
						if (key.key !== "Enter") return;
						key.preventDefault();
						answer("done");
					}}
				/>
			)}
			<div className="mt-2.5 flex flex-wrap items-center gap-1.5">
				{onOpenScreen !== undefined && (
					<button type="button" className="control btn-primary" onClick={onOpenScreen}>
						Open the screen
					</button>
				)}
				<button
					type="button"
					disabled={answering}
					className={onOpenScreen !== undefined ? "control btn" : "control btn-primary"}
					onClick={() => answer("done")}
				>
					Done
				</button>
				<button type="button" disabled={answering} className="control btn" onClick={() => answer("declined")}>
					Decline
				</button>
				{!noting && (
					<button type="button" className="control btn-quiet ml-auto" onClick={() => setNoting(true)}>
						Add a note
					</button>
				)}
			</div>
		</div>
	);
}

const PASSKEY_AFTERLIFE: Record<PasskeyAskStatus, string> = {
	pending: "Needs you",
	approved: "Approved",
	denied: "Denied",
	expired: "Expired",
};

/**
 * A site asked the teammate's browser to make a passkey, under an arming
 * the person started in the teammate's pane; the request waits in the
 * browser until it is answered here, from whichever seat. Approve and the
 * browser makes it, the room stores it under the arming's name and ticks
 * it for this teammate; deny and the site hears no and the arming ends. A
 * decided card is the outcome on the tape.
 */
function PasskeyAskCard({ personaId, event }: { personaId: string; event: Extract<TranscriptEvent, { kind: "passkey_ask" }> }) {
	const [answering, setAnswering] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const who = askedFor(event);
	const site = event.rpName !== undefined ? `${event.rpName} (${event.rpId})` : event.rpId;
	const asks = `${site} asks to make a passkey${who !== null ? ` for ${who}` : ""}.`;

	const answer = (approved: boolean) => {
		if (answering || event.status !== "pending") return;
		setAnswering(true);
		setRefusal(null);
		void wire.command("secrets.passkey.answer", { personaId, askId: event.askId, approved }).catch((error: unknown) => {
			setRefusal(error instanceof Error ? error.message : String(error));
			setAnswering(false);
		});
	};

	if (event.status !== "pending") {
		return (
			<div className="card mt-3">
				<p className="eyebrow mb-1">Passkey · {PASSKEY_AFTERLIFE[event.status]}</p>
				<p className="selectable text-ink-2">{asks}</p>
				{event.status === "approved" && (
					<p className="selectable mt-1 text-ink-2">
						Stored as <span className="font-mono">{event.name}</span> and ticked for this teammate once made.
					</p>
				)}
			</div>
		);
	}

	return (
		<div className="card card-live mt-3">
			<p className="eyebrow mb-1">Needs you · Passkey</p>
			<p className="selectable">{asks}</p>
			<p className="mt-1.5 text-sm text-ink-3">
				The request came from {event.origin}. Approved, the passkey is stored as <span className="font-mono">{event.name}</span> and
				ticked for this teammate, whose browser signs in there with it from now on. Denied, the site hears no and the arming ends.
			</p>
			<div className="mt-2.5 flex flex-wrap items-center gap-1.5">
				<button type="button" disabled={answering} className="control btn-primary" onClick={() => answer(true)}>
					Approve
				</button>
				<button type="button" disabled={answering} className="control btn" onClick={() => answer(false)}>
					Deny
				</button>
			</div>
			{refusal !== null && <p className="mt-2 text-sm text-danger">{refusal}</p>}
		</div>
	);
}

const EXCHANGE_SETTLED: Record<Exclude<ExchangePauseStatus, "pending">, string> = {
	resumed: "Exchange resumed",
	stopped: "Exchange stopped",
};

/**
 * Both asks and handoffs count toward the pair's message cap. Keep going
 * releases queued work and starts counting afresh; Stop exchange ends the
 * exchange, not the collaboration grant. Either outcome becomes a quiet line.
 */
function ExchangePaused({
	personaId,
	ownerName,
	event,
}: {
	personaId: string;
	event: Extract<TranscriptEvent, { kind: "exchange_paused" }>;
	ownerName: string;
}) {
	const [answering, setAnswering] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	if (event.status !== "pending") {
		return <p className="rule-line rule-line-plain">{EXCHANGE_SETTLED[event.status]}</p>;
	}

	const act = (cmd: "teammates.exchange_resume" | "teammates.exchange_stop") => {
		if (answering) return;
		setAnswering(true);
		setRefusal(null);
		void wire.command(cmd, { a: personaId, b: event.withPersonaId }).catch(() => {
			setAnswering(false);
			setRefusal("Could not update this exchange. Try again.");
		});
	};

	return (
		<div className="card card-live mt-3">
			<p className="eyebrow mb-1">Paused</p>
			<p className="selectable">
				{ownerName} and {event.withName} paused after {event.exchanges} {event.exchanges === 1 ? "message" : "messages"} without you.
			</p>
			<div className="card-actions">
				<button type="button" disabled={answering} className="control btn-primary" onClick={() => act("teammates.exchange_resume")}>
					Keep going
				</button>
				<button type="button" disabled={answering} className="control btn" onClick={() => act("teammates.exchange_stop")}>
					Stop exchange
				</button>
			</div>
			{refusal !== null && <p role="alert" className="mt-2 text-sm text-danger">{refusal}</p>}
		</div>
	);
}

/**
 * What the computer looked like. A thumbnail beside the words, because a
 * capture is evidence and not another message; pressed, it opens in the
 * viewer at a size that can be read.
 */
function ComputerFrame({ dataUrl }: { dataUrl: string }) {
	const [open, setOpen] = useState(false);
	return (
		<>
			<button
				type="button"
				className="picture-open mt-2 block w-28 overflow-hidden rounded-md border border-line bg-raised p-0 text-left"
				title="Open the capture"
				onClick={() => setOpen(true)}
			>
				<img src={dataUrl} alt="The computer's screen at capture" className="block w-full" />
			</button>
			{open && <Viewer src={dataUrl} alt="The computer's screen at capture" onClose={() => setOpen(false)} />}
		</>
	);
}

function firstLine(text: string): string {
	const line = text.trim().split("\n", 1)[0] ?? "";
	return line.length > 70 ? `${line.slice(0, 70)}…` : line;
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
export function runTime(at: number): string {
	const when = new Date(at);
	return daysBetween(when, new Date()) === 0 ? clock.format(when) : `${shortDate.format(when)}, ${clock.format(when)}`;
}

/** Calendar days apart, not elapsed hours: 11pm and 1am are a day apart. */
function daysBetween(then: Date, now: Date): number {
	const midnight = (date: Date) => new Date(date.getFullYear(), date.getMonth(), date.getDate());
	return Math.round((midnight(now).getTime() - midnight(then).getTime()) / 86_400_000);
}
