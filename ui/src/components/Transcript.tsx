import { useEffect, useRef, useState, type RefObject } from "react";
import type {
	HumanActionStatus,
	PermissionOption,
	PlanEntry,
	ToolOutput,
	ToolStatus,
	TranscriptEvent,
} from "../generated/contract";
import type { Streaming } from "../tape";
import { wire } from "../wire";
import { Markdown } from "./Markdown";

/** Long enough that a stamp means "we picked this back up later". */
const STAMP_AFTER = 20 * 60_000;
/** Slack under the latest line that still counts as following the conversation. */
const PIN_SLACK = 80;

/** A message being answered: the id the wire stamps, the line the chip shows. */
export type ReplyTarget = { eventId: string; text: string };

/**
 * The conversation, and only the conversation.
 *
 * The machinery an agent runs on — its thoughts, the tools it called — is
 * folded away rather than hidden: a transcript that reads as a chat is the
 * whole point, and a transcript that lies about what happened is not worth
 * having. One press opens either. An agent's line, hovered or focused, offers
 * a quiet Reply; a user line that answers one quotes the original from this
 * fold, and a missing original is not drawn.
 *
 * Imported tapes also hold permission cards, plans, peer markers, hands-to-
 * human and computer frames. A card with no decision is live: answering it
 * writes through the tape, so the buttons go away when the room supersedes
 * the line. A decided card, including one that expired, names the outcome.
 */
export function Transcript({
	personaId,
	events,
	streaming,
	focus,
	onReply,
}: {
	personaId: string;
	events: TranscriptEvent[];
	streaming: Streaming[];
	/** A search hit to land on. `at` is a nonce so picking the same id twice still jumps. */
	focus: { eventId: string; at: number } | null;
	onReply(target: ReplyTarget): void;
}) {
	const scroller = useRef<HTMLDivElement>(null);
	/* Following the conversation is the default and stays true until you
	 * scroll away from the bottom yourself. */
	const pinned = useRef(true);
	/* The empty state has no scroller at all, so the first event is what
	 * mounts one — and the listeners have to go on when it appears, not once
	 * at mount. */
	const empty = events.length === 0 && streaming.length === 0;
	/* A reply quote is the same jump as a search hit, asked for from inside
	 * the transcript rather than from the drawer. The later `at` wins so a
	 * tap after a search still lands. */
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
	}, [empty]);

	if (empty) {
		return (
			<div className="flex flex-1 items-center justify-center px-6">
				<p className="max-w-sm text-center text-ink-3">
					No messages yet. Say something to get started.
				</p>
			</div>
		);
	}

	return (
		<div ref={scroller} className="flex-1 overflow-y-auto px-6 py-5">
			{/* `justify-end` rests a short conversation on the composer rather
			    than stranding it at the top of an empty pane. */}
			<div className="mx-auto flex min-h-full w-full max-w-[46rem] flex-col justify-end gap-1.5">
				{events.map((event, index) => (
					<Line
						key={event.id}
						personaId={personaId}
						event={event}
						previous={events[index - 1]}
						said={said}
						onReply={onReply}
						onJump={(eventId) => setJumped({ eventId, at: Date.now() })}
					/>
				))}
				{streaming.map((one) =>
					one.kind === "agent" ? (
						<div key={one.messageId} className="flex">
							<div className="bubble bubble-them">
								<Markdown text={one.text} />
							</div>
						</div>
					) : (
						<Thought key={one.messageId} text={one.text} />
					),
				)}
			</div>
		</div>
	);
}

function Line({
	personaId,
	event,
	previous,
	said,
	onReply,
	onJump,
}: {
	personaId: string;
	event: TranscriptEvent;
	previous: TranscriptEvent | undefined;
	said: Map<string, string>;
	onReply(target: ReplyTarget): void;
	onJump(eventId: string): void;
}) {
	const stamp =
		event.kind !== "chapter" &&
		previous?.kind !== "chapter" &&
		(previous === undefined || event.ts - previous.ts > STAMP_AFTER);

	return (
		<div data-event-id={event.id} className="rounded-lg">
			{stamp && <p className="py-2 text-center text-xs text-ink-3">{stampText(event.ts)}</p>}
			<Row personaId={personaId} event={event} said={said} onReply={onReply} onJump={onJump} />
		</div>
	);
}

/**
 * A search hit or a reply's quote: unpin, bring the row to the middle, and
 * light it briefly.
 *
 * The tape arrives after the jump is asked for when Everywhere opens another
 * teammate, so this waits until the fold contains the id. `found` going true
 * is the retry; a later append does not change `found` and so does not jump.
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
		const row = root?.querySelector<HTMLElement>(`[data-event-id="${CSS.escape(focus.eventId)}"]`);
		if (!root || !row) return;
		pinned.current = false;
		const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
		row.scrollIntoView({ block: "center", behavior: reduce ? "auto" : "smooth" });
		row.classList.add("row-lit");
		const timer = window.setTimeout(() => row.classList.remove("row-lit"), 1_600);
		return () => window.clearTimeout(timer);
	}, [focus, found, scroller, pinned]);
}

function Row({
	personaId,
	event,
	said,
	onReply,
	onJump,
}: {
	personaId: string;
	event: TranscriptEvent;
	said: Map<string, string>;
	onReply(target: ReplyTarget): void;
	onJump(eventId: string): void;
}) {
	switch (event.kind) {
		case "user":
			return <UserBubble event={event} said={said} onJump={onJump} />;

		case "agent":
			return <AgentBubble event={event} onReply={onReply} />;

		case "thought":
			return <Thought text={event.text} />;

		case "tool":
			return <Tool title={event.title} status={event.status} output={event.output} />;

		/* Where the turn stopped. Quiet, because the agent finishing is the
		 * expected thing and only its manner is news. */
		case "turn":
			return (
				<p className="py-1.5 text-center text-xs text-ink-3">
					{event.stopReason.replace(/_/g, " ")}
					{event.usage?.totalTokens !== undefined && ` · ${event.usage.totalTokens} tokens`}
				</p>
			);

		case "notice":
			return (
				<p
					className="py-1 text-center text-xs"
					style={{ color: `var(--${event.level === "info" ? "ink-3" : event.level === "warn" ? "warn" : "danger"})` }}
				>
					{event.text}
				</p>
			);

		/* Where the agent's working context reset: the date stamp's own line,
		 * with the chapter's name on it once it has one. The close arrives as
		 * one superseded marker — endedAt and title together — so there is no
		 * interim "writing the note" to draw. Nothing to click; the note lives
		 * in search. */
		case "chapter":
			return (
				<p className="py-3 text-center text-xs text-ink-3">
					{stampText(event.ts)}
					{event.title !== undefined && event.title !== "" && (
						<span className="ml-2 text-ink-2">{event.title}</span>
					)}
				</p>
			);

		/* Open cards answer through the wire; the tape then supersedes the
		 * line, so a decided card is just the outcome and not another click. */
		case "permission":
			return <Permission personaId={personaId} event={event} />;

		case "plan":
			return <Plan entries={event.entries} />;

		case "human_action":
			return <HumanAction reason={event.reason} status={event.status} />;

		/* One quiet line, the way a chapter is a date: the name, who started
		 * it, how many turns, and whether it is still open. A click that
		 * opened the thread belongs to a window that has threads. */
		case "peer":
			return (
				<p className="py-3 text-center text-xs text-ink-3">
					with {event.seat === "client" ? `${event.withName} (an outside agent)` : event.withName}
					<span className="ml-2">
						{event.role} · {event.exchanges === 1 ? "1 exchange" : `${event.exchanges} exchanges`} ·{" "}
						{event.status}
					</span>
				</p>
			);

		case "computer_frame":
			return <ComputerFrame dataUrl={event.dataUrl} />;
	}
}

/** An agent's line, focusable so R can answer it without a pointer. */
function AgentBubble({
	event,
	onReply,
}: {
	event: Extract<TranscriptEvent, { kind: "agent" }>;
	onReply(target: ReplyTarget): void;
}) {
	const reply = () => onReply({ eventId: event.id, text: firstLine(event.text) });
	return (
		<div className="group-msg relative mt-2 flex">
			<div
				className="bubble bubble-them"
				tabIndex={0}
				onKeyDown={(key) => {
					if (key.repeat) return;
					if (key.ctrlKey || key.altKey || key.metaKey) return;
					if (key.key !== "r" && key.key !== "R") return;
					key.preventDefault();
					reply();
				}}
			>
				<Markdown text={event.text} />
			</div>
			<button type="button" className="reply-affordance" tabIndex={-1} title="Reply (R)" onClick={reply}>
				Reply
			</button>
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
		<div className="mt-2 flex justify-end">
			<div className="bubble bubble-me">
				{quote !== undefined && answered !== undefined && (
					<button
						type="button"
						className="bubble-quote"
						title="Go to the message"
						onClick={() => onJump(answered)}
					>
						{quote}
					</button>
				)}
				{text}
				{event.attachments !== undefined && event.attachments.length > 0 && (
					<ul className="bubble-files">
						{event.attachments.map((item) => (
							<li key={item.path} className="bubble-file" title={item.path}>
								{item.name}
							</li>
						))}
					</ul>
				)}
			</div>
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

/** What the agent was thinking, folded away. */
function Thought({ text }: { text: string }) {
	const [open, setOpen] = useState(false);
	return (
		<div className="my-0.5 max-w-[85%]">
			<button type="button" className="aside" aria-expanded={open} onClick={() => setOpen((was) => !was)}>
				thought{open ? "" : ` · ${firstLine(text)}`}
			</button>
			{open && (
				<pre className="mt-1 whitespace-pre-wrap rounded-lg bg-paper-3 px-2.5 py-2 font-mono text-xs text-ink-2">
					{text}
				</pre>
			)}
		</div>
	);
}

const STATUS_INK: Record<ToolStatus, string> = {
	pending: "var(--ink-3)",
	in_progress: "var(--accent)",
	completed: "var(--ink-3)",
	failed: "var(--danger)",
};

/** A tool call: what it was, how it went, and its output behind a press. */
function Tool({
	title,
	status,
	output,
}: {
	title: string;
	status: ToolStatus;
	output: ToolOutput[] | undefined;
}) {
	const [open, setOpen] = useState(false);
	const hasOutput = output !== undefined && output.length > 0;
	return (
		<div className="my-0.5 max-w-[85%]">
			<button
				type="button"
				className="aside flex items-center gap-2"
				aria-expanded={hasOutput ? open : undefined}
				disabled={!hasOutput}
				onClick={() => setOpen((was) => !was)}
			>
				<span
					aria-hidden="true"
					className={`h-1.5 w-1.5 shrink-0 rounded-full ${status === "in_progress" ? "animate-throat" : ""}`}
					style={{ background: STATUS_INK[status] }}
				/>
				<span className="truncate font-mono">{title}</span>
				<span className="shrink-0">{status.replace(/_/g, " ")}</span>
			</button>
			{open && hasOutput && (
				<pre className="mt-1 max-h-80 overflow-auto whitespace-pre-wrap rounded-lg bg-paper-3 px-2.5 py-2 font-mono text-xs text-ink-2">
					{output.map(outputText).join("\n\n")}
				</pre>
			)}
		</div>
	);
}

function outputText(one: ToolOutput): string {
	return one.type === "text" ? one.text : `${one.path}\n${one.newText}`;
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
			.command("session.answer_permission", {
				personaId,
				requestId: event.requestId,
				optionId,
			})
			.catch(() => setAnswering(false));
	};

	return (
		<div className="tape-card">
			<p>{event.title}</p>
			{chosen !== undefined ? (
				<p className="mt-1 text-xs text-ink-3">{chosen}</p>
			) : (
				<div className="ask-actions">
					{event.options.map((option) => (
						<button
							key={option.optionId}
							type="button"
							disabled={answering}
							className={optionKindClass(option)}
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
	if (event.decision === "expired") return "Expired";
	if (event.decidedOptionName !== undefined && event.decidedOptionName !== "") {
		return event.decidedOptionName;
	}
	if (event.decision === undefined) return undefined;
	const option = event.options.find((one) => one.optionId === event.decision);
	return option?.name ?? event.decision;
}

function optionKindClass(option: PermissionOption): string {
	return option.kind?.startsWith("allow") ? "btn-primary" : "btn-quiet";
}

/** The agent's working list, one status per line. */
function Plan({ entries }: { entries: PlanEntry[] }) {
	if (entries.length === 0) return null;
	return (
		<ul className="plan-list">
			{entries.map((entry, index) => (
				<li key={`${index}:${entry.content}`} className="plan-row">
					<span className="plan-mark" aria-hidden="true">
						{planMark(entry.status)}
					</span>
					<span className="min-w-0 flex-1">{entry.content}</span>
					<span className="plan-status">{entry.status.replace(/_/g, " ")}</span>
				</li>
			))}
		</ul>
	);
}

function planMark(status: string): string {
	if (status === "completed") return "✓";
	if (status === "in_progress") return "…";
	return "·";
}

/** The agent asked for hands it does not have. Status is the whole afterlife. */
function HumanAction({ reason, status }: { reason: string; status: HumanActionStatus }) {
	return (
		<div className="tape-card">
			<p className="mb-1 text-xs uppercase tracking-wide text-ink-3">{status}</p>
			<p>{reason}</p>
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
			className="frame-thumb"
			aria-expanded={open}
			title={open ? "Hide the capture" : "Show the capture"}
			onClick={() => setOpen((was) => !was)}
			onKeyDown={(key) => {
				if (key.key !== "Escape" || !open) return;
				key.preventDefault();
				setOpen(false);
			}}
		>
			<img src={dataUrl} alt="The computer's screen at capture" />
		</button>
	);
}

function firstLine(text: string): string {
	const line = text.trim().split("\n", 1)[0] ?? "";
	return line.length > 70 ? `${line.slice(0, 70)}…` : line;
}

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const weekday = new Intl.DateTimeFormat(undefined, {
	weekday: "short",
	month: "short",
	day: "numeric",
});

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
