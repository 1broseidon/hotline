import { ErrorCard } from "./ErrorCard";
import { useEffect, useReducer, useRef, useState, type RefObject } from "react";
import type {
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
import { ArrowDownIcon, CheckIcon, ChevronDownIcon, ChevronRightIcon, ClockIcon, ReplyIcon, WarningIcon } from "../icons";
import type { Streaming } from "../tape";
import { activityOf } from "../activity";
import { Glyph } from "../ui/Glyph";
import { Avatar } from "../ui/Avatar";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import { Markdown } from "./Markdown";
import { askedFor } from "./PasskeyArm";

/** Long enough that a stamp means "we picked this back up later". */
const STAMP_AFTER = 20 * 60_000;
/** Slack under the latest line that still counts as following the conversation. */
const PIN_SLACK = 80;

/** A message being answered: the id the wire stamps, the line the chip shows. */
export type ReplyTarget = { eventId: string; text: string };

/** Whose chair we are in, for a peer thread: this teammate is `mine`. */
export type Speakers = { me: string; them: string; mine: "user" | "agent" };

type Step = Extract<TranscriptEvent, { kind: "thought" | "tool" }>;

/**
 * Either one event, or a run of the machinery between two messages —
 * thoughts and tool calls — folded into one block so a transcript of forty
 * tool calls still reads as a conversation.
 */
type Block =
	| { kind: "event"; event: Exclude<TranscriptEvent, Step> }
	| { kind: "steps"; id: string; ts: number; items: Step[] };

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
	events,
	streaming,
	live,
	focus,
	speakers,
	onReply,
	onOpenThread,
	onOpenScreen,
}: {
	personaId: string;
	name: string;
	events: TranscriptEvent[];
	streaming: Streaming[];
	/** A turn is running: the mark is up, above the composer. */
	live: boolean;
	/** A search hit to land on. `at` is a nonce so picking the same id twice still jumps. */
	focus: { eventId: string; at: number } | null;
	/** A peer thread names both sides; the tape with the person does not. */
	speakers?: Speakers;
	onReply?(target: ReplyTarget): void;
	onOpenThread?(event: Extract<TranscriptEvent, { kind: "peer" }>): void;
	/** The teammate's desktop, only while one is running: opens it in a window of its own. */
	onOpenScreen?(): void;
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

	const said = new Map<string, string>();
	for (const event of events) {
		const line = quotedLine(event);
		if (line !== undefined) said.set(event.id, line);
	}

	useScrollToEvent(scroller, pinned, landing, events);

	useEffect(() => {
		const el = scroller.current;
		if (!el) return;
		const measure = () => {
			pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < PIN_SLACK;
			setFollowing(pinned.current);
		};
		const pin = () => {
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
	const arrived = toBlocks(events, streaming);
	const hidden = useCadence(personaId, arrived);

	if (empty) {
		return (
			<div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 pb-16">
				<Avatar id={personaId} name={name} size={48} />
				<p className="text-lg font-semibold">{name}</p>
				<p className="text-center text-sm text-ink-3">Nothing said yet. Say hello below.</p>
			</div>
		);
	}

	const blocks = hidden.size === 0 ? arrived : arrived.filter((block) => !(block.kind === "event" && hidden.has(block.event.id)));
	const activity = live || hidden.size > 0 ? activityOf(events, streaming, hidden.size > 0) : null;
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
			    than stranding it at the top of an empty pane. */}
			<div className={`mx-auto flex min-h-full w-full max-w-[46rem] flex-col justify-end px-6 pt-6 ${live ? "pb-14" : "pb-6"}`}>
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
						<div key={id} data-event-id={id}>
							{stamp && <p className="rule-line rule-line-plain">{stampText(block_ts(block))}</p>}
							{block.kind === "steps" ? (
								<Steps items={block.items} live={live && index === blocks.length - 1} shown={workShown} />
							) : (
								<Row
									personaId={personaId}
									event={block.event}
									said={said}
									run={run}
									speakers={speakers}
									{...(onReply !== undefined ? { onReply } : {})}
									{...(onOpenThread !== undefined ? { onOpenThread } : {})}
									{...(onOpenScreen !== undefined ? { onOpenScreen } : {})}
									onJump={(eventId) => setJumped({ eventId, at: Date.now() })}
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
						aria-expanded={workShown}
						title={workShown ? "Hide the work" : "Show the work"}
						onClick={() => setWorkShown((was) => !was)}
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
				className="control btn send absolute bottom-3 right-8"
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
function toBlocks(events: TranscriptEvent[], streaming: Streaming[]): Block[] {
	const blocks: Block[] = [];
	for (const event of events) {
		if (event.kind === "thought" || event.kind === "tool") {
			const tail = blocks[blocks.length - 1];
			if (tail?.kind === "steps") tail.items.push(event);
			else blocks.push({ kind: "steps", id: event.id, ts: event.ts, items: [event] });
		} else {
			blocks.push({ kind: "event", event });
		}
	}
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
		if (tail?.kind === "steps") tail.items.push(thought);
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

function Row({
	personaId,
	event,
	said,
	run,
	speakers,
	onReply,
	onOpenThread,
	onOpenScreen,
	onJump,
}: {
	personaId: string;
	event: Exclude<TranscriptEvent, Step>;
	said: Map<string, string>;
	run: Run;
	speakers: Speakers | undefined;
	onReply?(target: ReplyTarget): void;
	onOpenThread?(event: Extract<TranscriptEvent, { kind: "peer" }>): void;
	onOpenScreen?(): void;
	onJump(eventId: string): void;
}) {
	switch (event.kind) {
		case "user":
			return event.scheduled !== undefined ? (
				<ScheduledLine name={event.scheduled.name} prompt={event.text} />
			) : speakers !== undefined ? (
				<NamedSay name={speakers.mine === "user" ? speakers.me : speakers.them} mine={speakers.mine === "user"} text={event.text} />
			) : (
				<UserBubble event={event} said={said} run={run} onJump={onJump} />
			);

		case "agent":
			return speakers !== undefined ? (
				<NamedSay
					name={speakers.mine === "agent" ? speakers.me : speakers.them}
					mine={speakers.mine === "agent"}
					text={event.text}
				/>
			) : (
				<AgentSay event={event} run={run} {...(onReply !== undefined ? { onReply } : {})} />
			);

		/* Where the turn stopped. Drawn only when the stop was not the agent's
		 * own choice; a count of tokens is the harness's business, not the
		 * conversation's. */
		case "turn":
			if (event.stopReason === "end_turn") return null;
			return <p className="instrument mt-1 text-right text-ink-4">{event.stopReason.replace(/_/g, " ")}</p>;

		case "notice":
			if (event.level === "error") return <ErrorCard text={event.text} />;
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
			return <Permission personaId={personaId} event={event} />;

		case "plan":
			return <Plan entries={event.entries} />;

		case "human_action":
			return <HumanAction personaId={personaId} event={event} {...(onOpenScreen !== undefined ? { onOpenScreen } : {})} />;

		case "passkey_ask":
			return <PasskeyAskCard personaId={personaId} event={event} />;

		/* One quiet line, the way a chapter is a date. Pressing it opens
		 * the thread in the inspector's place. */
		case "peer":
			return (
				<button type="button" className="rule-line rule-line-plain w-full" onClick={() => onOpenThread?.(event)}>
					{[
						`With ${event.seat === "client" ? `${event.withName} (outside the room)` : event.withName}`,
						event.role,
						event.exchanges === 1 ? "1 exchange" : `${event.exchanges} exchanges`,
						event.status,
					].join(" · ")}
				</button>
			);

		case "computer_frame":
			return <ComputerFrame dataUrl={event.dataUrl} />;

		case "computer_pull":
			return <PullLine event={event} />;
	}
}

/**
 * An image pull as one line that fills in. The runtime names each layer as
 * it starts and finishes it, and the desk rewrites this line under one id
 * as they land, so a minute of download reads as a bar rather than a
 * silence; a runtime the desk cannot count leaves the bar indeterminate.
 * Done, the line says what came down and how long it took. Failed, it says
 * so, and the error that ended the start says why.
 */
function PullLine({ event }: { event: Extract<TranscriptEvent, { kind: "computer_pull" }> }) {
	const short = shortImage(event.image);
	if (event.status === "done") {
		const took = event.elapsedMs === undefined ? "" : ` in ${tookWords(event.elapsedMs)}`;
		return (
			<p className="rule-line rule-line-plain gap-1.5" style={{ color: "var(--ink-3)" }}>
				<span className="selectable font-normal">
					Pulled {short}
					{took}
				</span>
			</p>
		);
	}
	if (event.status === "failed") {
		return (
			<p className="rule-line rule-line-plain gap-1.5" style={{ color: "var(--warn)" }}>
				<WarningIcon className="shrink-0" />
				<span className="selectable font-normal">The pull of {short} did not finish</span>
			</p>
		);
	}
	const counted = event.layersTotal > 0;
	return (
		<div className="rule-line rule-line-plain flex-col items-stretch gap-1.5" style={{ color: "var(--ink-3)" }} role="status">
			<span className="flex justify-between gap-3">
				<span className="selectable font-normal">Pulling {short}…</span>
				{counted && (
					<span className="tabular-nums">
						{event.layersDone} / {event.layersTotal} layers
					</span>
				)}
			</span>
			<progress
				className="w-full accent-[var(--accent)]"
				aria-label={`Pulling ${short}`}
				max={counted ? event.layersTotal : undefined}
				value={counted ? event.layersDone : undefined}
			/>
		</div>
	);
}

/** `ghcr.io/1broseidon/hotline-computer:0.9.1` → `hotline-computer 0.9.1`. */
function shortImage(image: string): string {
	const tail = image.slice(image.lastIndexOf("/") + 1);
	const at = tail.indexOf(":");
	return at === -1 ? tail : `${tail.slice(0, at)} ${tail.slice(at + 1)}`;
}

function tookWords(ms: number): string {
	return ms < 1000 ? "under a second" : `${Math.round(ms / 1000)} s`;
}

/**
 * The machinery between two messages, as one caption: a count, closed
 * until you open it. While the agent is still on it there is no caption at
 * all — the typing bubble is the caption — and the rows show only if you
 * pressed that bubble to see the work.
 */
function Steps({ items, live, shown }: { items: Step[]; live: boolean; shown: boolean }) {
	const [toggled, setToggled] = useState(false);
	const open = live ? shown : toggled;
	const failed = items.some((one) => one.kind === "tool" && one.status === "failed");
	const summary = `${items.length} ${items.length === 1 ? "step" : "steps"}${failed ? " · one failed" : ""}`;

	return (
		<div className="my-2">
			{!live && (
				<button type="button" className="steps-caption" aria-expanded={open} onClick={() => setToggled(!open)}>
					<span className={`truncate ${failed ? "text-danger" : ""}`}>{summary}</span>
					{open ? <ChevronDownIcon /> : <ChevronRightIcon />}
				</button>
			)}
			{open && (
				<div className="steps ml-[11px]">
					{items.map((item) =>
						item.kind === "thought" ? (
							<Thought key={item.id} id={item.id} text={item.text} />
						) : (
							<Tool key={item.id} id={item.id} title={item.title} status={item.status} output={item.output} />
						),
					)}
				</div>
			)}
		</div>
	);
}

/** An agent's line, focusable so R can answer it without a pointer. */
function AgentSay({
	event,
	run,
	onReply,
}: {
	event: Extract<TranscriptEvent, { kind: "agent" }>;
	run: Run;
	onReply?(target: ReplyTarget): void;
}) {
	const reply = () => onReply?.({ eventId: event.id, text: firstLine(event.text) });
	return (
		<div className={`said-group relative ${run.top ? "mt-1" : "mt-3"}`}>
			<div
				className={`speech said-them ${runClass(run)}`}
				tabIndex={onReply === undefined ? undefined : 0}
				onKeyDown={(key) => {
					if (onReply === undefined) return;
					if (key.repeat) return;
					if (key.ctrlKey || key.altKey || key.metaKey) return;
					if (key.key !== "r" && key.key !== "R") return;
					key.preventDefault();
					reply();
				}}
			>
				<Markdown text={event.text} />
				<Reactions emoji={event.reactions} />
				{onReply !== undefined && (
					<button
						type="button"
						className="reply-affordance control btn btn-sm gap-1"
						tabIndex={-1}
						title={`Reply (${chordKeys("reply")})`}
						onClick={reply}
					>
						<ReplyIcon />
						Reply
					</button>
				)}
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
	said,
	run,
	onJump,
}: {
	event: Extract<TranscriptEvent, { kind: "user" }>;
	said: Map<string, string>;
	run: Run;
	onJump(eventId: string): void;
}) {
	const answered = event.replyTo;
	const quote = answered !== undefined ? said.get(answered) : undefined;
	const text = quote !== undefined ? unquoted(event.text) : event.text;
	return (
		<div className={`flex justify-end ${run.top ? "mt-1" : "mt-3"}`}>
			<div className={`speech said-me ${runClass(run)}`}>
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
				<Reactions emoji={event.reactions} />
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

/** First line of a say, or nothing — a missing or empty original is not a quote. */
function quotedLine(event: TranscriptEvent): string | undefined {
	if (event.kind !== "user" && event.kind !== "agent") return undefined;
	const line = firstLine(event.text);
	return line.length > 0 ? line : undefined;
}

/** The stored text minus a leading quote block the old composer prepended. */
function unquoted(text: string): string {
	const lines = text.split("\n");
	let end = 0;
	while (end < lines.length && lines[end]!.startsWith(">")) end++;
	if (end === 0) return text;
	return lines.slice(end).join("\n").replace(/^\n+/, "");
}

/** What the agent was thinking, one line until pressed. */
function Thought({ id, text }: { id: string; text: string }) {
	const [open, setOpen] = useState(false);
	return (
		<div data-step-id={id}>
			<button type="button" className="step" aria-expanded={open} onClick={() => setOpen((was) => !was)}>
				<span aria-hidden="true" className="step-mark" style={{ boxShadow: "inset 0 0 0 1.5px var(--ink-4)" }} />
				<span className="step-title font-sans italic text-ink-3">{firstLine(text)}</span>
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
	const hasOutput = output !== undefined && output.length > 0;
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
				<span className="step-title">{title}</span>
				{status === "failed" && <span className="shrink-0 text-danger">failed</span>}
				{status === "in_progress" && <span className="shrink-0 text-ink-3">running</span>}
				{hasOutput && (open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />)}
			</button>
			{open && hasOutput && <div className="step-out">{output.map(outputText).join("\n\n")}</div>}
		</div>
	);
}

function outputText(one: ToolOutput): string {
	if (one.type === "text") return one.text;
	const before = one.oldText == null ? [] : one.oldText.split("\n").map((line) => `- ${line}`);
	const after = one.newText.split("\n").map((line) => `+ ${line}`);
	return [one.path, ...before, ...after].join("\n");
}

/**
 * A permission the agent asked. The choice is history when the tape already
 * names it; otherwise the options are the answer, and they stay dead while
 * that answer is in flight so a second click cannot race the first.
 */
function Permission({
	personaId,
	event,
}: {
	personaId: string;
	event: Extract<TranscriptEvent, { kind: "permission" }>;
}) {
	const [answering, setAnswering] = useState(false);
	const chosen = chosenOption(event);

	const answer = (optionId: string) => {
		if (answering || event.decision !== undefined) return;
		setAnswering(true);
		void wire
			.command("session.answer_permission", { personaId, requestId: event.requestId, optionId })
			.catch(() => setAnswering(false));
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
				<p className="eyebrow mb-1">Needed you · {AFTERLIFE[event.status]}</p>
				<p className="selectable text-ink-2">{event.reason}</p>
				{event.note ? <p className="selectable mt-1 text-ink-2">You said: {event.note}</p> : null}
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

/**
 * What the computer looked like. A thumbnail until asked for, because a
 * capture is evidence beside the words, not another message. Escape puts
 * it back when the button still has focus.
 */
function ComputerFrame({ dataUrl }: { dataUrl: string }) {
	const [open, setOpen] = useState(false);
	return (
		<button
			type="button"
			className="mt-2 block overflow-hidden rounded-md border border-line bg-raised p-0 text-left transition-[width]"
			style={{ width: open ? "min(24rem, 100%)" : "7rem" }}
			aria-expanded={open}
			title={open ? "Hide the capture" : "Show the capture"}
			onClick={() => setOpen((was) => !was)}
			onKeyDown={(key) => {
				if (key.key !== "Escape" || !open) return;
				key.preventDefault();
				setOpen(false);
			}}
		>
			<img src={dataUrl} alt="The computer's screen at capture" className="block w-full" />
		</button>
	);
}

function firstLine(text: string): string {
	const line = text.trim().split("\n", 1)[0] ?? "";
	return line.length > 70 ? `${line.slice(0, 70)}…` : line;
}

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short", month: "short", day: "numeric" });

function stampText(at: number): string {
	const when = new Date(at);
	const days = daysBetween(when, new Date());
	if (days === 0) return `Today ${clock.format(when)}`;
	if (days === 1) return `Yesterday ${clock.format(when)}`;
	return `${weekday.format(when)} ${clock.format(when)}`;
}

/** Calendar days apart, not elapsed hours: 11pm and 1am are a day apart. */
function daysBetween(then: Date, now: Date): number {
	const midnight = (date: Date) => new Date(date.getFullYear(), date.getMonth(), date.getDate());
	return Math.round((midnight(now).getTime() - midnight(then).getTime()) / 86_400_000);
}
