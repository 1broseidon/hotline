import { useEffect, useRef, useState, type RefObject } from "react";
import type {
	HumanActionStatus,
	HumanAnswer,
	PermissionOption,
	PlanEntry,
	ToolOutput,
	ToolStatus,
	TranscriptEvent,
} from "../generated/contract";
import { chordKeys } from "../chords";
import { ArrowDownIcon, CheckIcon, ChevronDownIcon, ChevronRightIcon, ClockIcon, ReplyIcon, WarningIcon } from "../icons";
import type { Streaming } from "../tape";
import { Avatar } from "../ui/Avatar";
import { wire } from "../wire";
import { Markdown } from "./Markdown";

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
 * An agent's words are set as text in the reading column; yours are the one
 * bubble, on the right. The machinery an agent runs on is folded, not
 * hidden: a block of steps opens while the agent is working and closes to a
 * count when its next message lands, and any row in it opens on a press.
 * A transcript that lies about what happened is not worth having.
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
}: {
	personaId: string;
	name: string;
	events: TranscriptEvent[];
	streaming: Streaming[];
	/** The session is thinking: the trailing block of steps stays open. */
	live: boolean;
	/** A search hit to land on. `at` is a nonce so picking the same id twice still jumps. */
	focus: { eventId: string; at: number } | null;
	/** A peer thread names both sides; the tape with the person does not. */
	speakers?: Speakers;
	onReply?(target: ReplyTarget): void;
	onOpenThread?(event: Extract<TranscriptEvent, { kind: "peer" }>): void;
}) {
	const scroller = useRef<HTMLDivElement>(null);
	/* Following the conversation is the default and stays true until you
	 * scroll away from the bottom yourself. */
	const pinned = useRef(true);
	/* The same fact, for the button that offers the way back down. */
	const [following, setFollowing] = useState(true);
	const empty = events.length === 0 && streaming.length === 0;
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

	if (empty) {
		return (
			<div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 pb-16">
				<Avatar id={personaId} name={name} size={48} />
				<p className="text-lg font-semibold">{name}</p>
				<p className="text-center text-sm text-ink-3">Nothing said yet. Start below.</p>
			</div>
		);
	}

	const blocks = toBlocks(events, streaming);
	const streamingSay = streaming.find((one) => one.kind === "agent");

	return (
		<div className="relative flex min-h-0 flex-1 flex-col">
		<div ref={scroller} className="flex-1 overflow-y-auto px-8 py-6">
			{/* `justify-end` rests a short conversation on the composer rather
			    than stranding it at the top of an empty pane. */}
			<div className="mx-auto flex min-h-full w-full max-w-[46rem] flex-col justify-end">
				{blocks.map((block, index) => {
					const previous = blocks[index - 1];
					const stamp =
						!isChapter(block) &&
						(previous === undefined || (!isChapter(previous) && block_ts(block) - block_ts(previous) > STAMP_AFTER));
					const id = block.kind === "event" ? block.event.id : block.id;
					return (
						<div key={id} data-event-id={id}>
							{stamp && <p className="rule-line rule-line-plain">{stampText(block_ts(block))}</p>}
							{block.kind === "steps" ? (
								<Steps
									items={block.items}
									live={live && index === blocks.length - 1 && streamingSay === undefined}
								/>
							) : (
								<Row
									personaId={personaId}
									event={block.event}
									said={said}
									speakers={speakers}
									{...(onReply !== undefined ? { onReply } : {})}
									{...(onOpenThread !== undefined ? { onOpenThread } : {})}
									onJump={(eventId) => setJumped({ eventId, at: Date.now() })}
								/>
							)}
						</div>
					);
				})}
				{streamingSay !== undefined && (
					<div className="said-group relative mt-3">
						<div className="speech said-them said-streaming">
							<Markdown text={streamingSay.text} />
							<span aria-hidden="true" className="beat ml-0.5 inline-block h-[14px] w-[2px] translate-y-[2px] bg-accent" />
						</div>
					</div>
				)}
			</div>
		</div>
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

/** Fold runs of thoughts and tools into one block; a streaming thought joins the tail. */
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
		if (one.kind !== "thought") continue;
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
	speakers,
	onReply,
	onOpenThread,
	onJump,
}: {
	personaId: string;
	event: Exclude<TranscriptEvent, Step>;
	said: Map<string, string>;
	speakers: Speakers | undefined;
	onReply?(target: ReplyTarget): void;
	onOpenThread?(event: Extract<TranscriptEvent, { kind: "peer" }>): void;
	onJump(eventId: string): void;
}) {
	switch (event.kind) {
		case "user":
			return event.scheduled !== undefined ? (
				<ScheduledLine name={event.scheduled.name} prompt={event.text} />
			) : speakers !== undefined ? (
				<NamedSay name={speakers.mine === "user" ? speakers.me : speakers.them} mine={speakers.mine === "user"} text={event.text} />
			) : (
				<UserBubble event={event} said={said} onJump={onJump} />
			);

		case "agent":
			return speakers !== undefined ? (
				<NamedSay name={speakers.mine === "agent" ? speakers.me : speakers.them} mine={speakers.mine === "agent"} text={event.text} />
			) : (
				<AgentSay event={event} {...(onReply !== undefined ? { onReply } : {})} />
			);

		/* Where the turn stopped. Drawn only when it says something the last
		 * message did not: a count, or a stop that was not the agent's choice. */
		case "turn": {
			const tokens = event.usage?.totalTokens;
			const ordinary = event.stopReason === "end_turn";
			if (ordinary && tokens === undefined) return null;
			return (
				<p className="mt-1 text-right text-xs text-ink-4">
					{!ordinary && <span className="text-ink-3">{event.stopReason.replace(/_/g, " ")}</span>}
					{!ordinary && tokens !== undefined && " · "}
					{tokens !== undefined && `${tokensText(tokens)} tokens`}
				</p>
			);
		}

		case "notice":
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
			return <HumanAction personaId={personaId} event={event} />;

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
	}
}

/**
 * The machinery between two messages. Open while the agent is on it, with
 * the latest step named in the summary; closed to a count once it has moved
 * on, unless you opened it yourself.
 */
function Steps({ items, live }: { items: Step[]; live: boolean }) {
	const [toggled, setToggled] = useState<boolean | null>(null);
	const open = toggled ?? live;
	const tools = items.filter((one) => one.kind === "tool").length;
	const thoughts = items.length - tools;
	const latest = items[items.length - 1];
	const failed = items.some((one) => one.kind === "tool" && one.status === "failed");
	const summary = live
		? latest?.kind === "tool"
			? latest.title
			: "Thinking"
		: [tools > 0 && `${tools} ${tools === 1 ? "tool" : "tools"}`, thoughts > 0 && `${thoughts} ${thoughts === 1 ? "thought" : "thoughts"}`]
				.filter(Boolean)
				.join(" · ");

	return (
		<div className="mt-2">
			<button
				type="button"
				className="step w-auto max-w-full"
				aria-expanded={open}
				onClick={() => setToggled(!open)}
			>
				<span
					aria-hidden="true"
					className={`step-mark ${live ? "beat" : ""}`}
					style={{ background: live ? "var(--accent)" : failed ? "var(--danger)" : "var(--ink-4)" }}
				/>
				<span className={`step-title ${live && latest?.kind === "tool" ? "" : "font-sans"}`}>
					{live && <span className="font-sans text-ink-3">Working · </span>}
					{summary}
				</span>
				{open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />}
			</button>
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
	onReply,
}: {
	event: Extract<TranscriptEvent, { kind: "agent" }>;
	onReply?(target: ReplyTarget): void;
}) {
	const reply = () => onReply?.({ eventId: event.id, text: firstLine(event.text) });
	return (
		<div className="said-group relative mt-3">
			<div
				className="speech said-them rounded-md"
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
			</div>
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
	onJump,
}: {
	event: Extract<TranscriptEvent, { kind: "user" }>;
	said: Map<string, string>;
	onJump(eventId: string): void;
}) {
	const answered = event.replyTo;
	const quote = answered !== undefined ? said.get(answered) : undefined;
	const text = quote !== undefined ? unquoted(event.text) : event.text;
	return (
		<div className="mt-3 flex justify-end">
			<div className="speech said-me">
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
			</div>
		</div>
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
			<p className="selectable">{event.title}</p>
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
 * here; a decided one is the outcome on the tape. Decline asks for a
 * one-line note in place — the wire has no field for it, so the note
 * stays on the screen and `human.answer` goes out as declined alone.
 */
function HumanAction({
	personaId,
	event,
}: {
	personaId: string;
	event: Extract<TranscriptEvent, { kind: "human_action" }>;
}) {
	const [answering, setAnswering] = useState(false);
	const [declining, setDeclining] = useState(false);
	const [note, setNote] = useState("");

	const answer = (status: HumanAnswer) => {
		if (answering || event.status !== "pending") return;
		setAnswering(true);
		void wire
			.command("human.answer", { personaId, actionId: event.actionId, status })
			.catch(() => setAnswering(false));
	};

	if (event.status !== "pending") {
		return (
			<div className="card mt-3">
				<p className="eyebrow mb-1">Needed you · {AFTERLIFE[event.status]}</p>
				<p className="selectable text-ink-2">{event.reason}</p>
			</div>
		);
	}

	return (
		<div className="card card-live mt-3">
			<p className="eyebrow mb-1">Needs you</p>
			<p className="selectable">{event.reason}</p>
			{declining ? (
				<form
					className="mt-2.5 flex items-center gap-1.5"
					onSubmit={(submit) => {
						submit.preventDefault();
						answer("declined");
					}}
				>
					<input
						className="field min-w-0 flex-1"
						aria-label="A note for the decline"
						placeholder="A one-line note"
						autoComplete="off"
						autoFocus
						value={note}
						onChange={(change) => setNote(change.target.value)}
						onKeyDown={(key) => {
							if (key.key !== "Escape") return;
							key.preventDefault();
							setDeclining(false);
						}}
					/>
					<button type="submit" disabled={answering} className="control btn btn-sm">
						Decline
					</button>
				</form>
			) : (
				<div className="card-actions">
					<button type="button" disabled={answering} className="control btn-primary" onClick={() => answer("done")}>
						Done
					</button>
					<button type="button" disabled={answering} className="control btn" onClick={() => setDeclining(true)}>
						Decline
					</button>
				</div>
			)}
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
			className="mt-2 block overflow-hidden rounded-md border border-line bg-inset p-0 text-left transition-[width]"
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

function tokensText(count: number): string {
	if (count < 1_000) return String(count);
	if (count < 100_000) return `${(count / 1_000).toFixed(1)}k`;
	return `${Math.round(count / 1_000)}k`;
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
