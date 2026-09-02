import { useEffect, useRef, useState, type RefObject } from "react";
import type { ToolOutput, ToolStatus, TranscriptEvent } from "../generated/contract";
import type { Streaming } from "../tape";
import { Markdown } from "./Markdown";

/** Long enough that a stamp means "we picked this back up later". */
const STAMP_AFTER = 20 * 60_000;
/** Slack under the latest line that still counts as following the conversation. */
const PIN_SLACK = 80;

/**
 * The conversation, and only the conversation.
 *
 * The machinery an agent runs on — its thoughts, the tools it called — is
 * folded away rather than hidden: a transcript that reads as a chat is the
 * whole point, and a transcript that lies about what happened is not worth
 * having. One press opens either.
 */
export function Transcript({
	events,
	streaming,
	focus,
}: {
	events: TranscriptEvent[];
	streaming: Streaming[];
	/** A search hit to land on. `at` is a nonce so picking the same id twice still jumps. */
	focus: { eventId: string; at: number } | null;
}) {
	const scroller = useRef<HTMLDivElement>(null);
	/* Following the conversation is the default and stays true until you
	 * scroll away from the bottom yourself. */
	const pinned = useRef(true);
	/* The empty state has no scroller at all, so the first event is what
	 * mounts one — and the listeners have to go on when it appears, not once
	 * at mount. */
	const empty = events.length === 0 && streaming.length === 0;

	useScrollToEvent(scroller, pinned, focus, events);

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
					<Line key={event.id} event={event} previous={events[index - 1]} />
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
	event,
	previous,
}: {
	event: TranscriptEvent;
	previous: TranscriptEvent | undefined;
}) {
	const stamp =
		event.kind !== "chapter" &&
		previous?.kind !== "chapter" &&
		(previous === undefined || event.ts - previous.ts > STAMP_AFTER);

	return (
		<div data-event-id={event.id} className="rounded-lg">
			{stamp && <p className="py-2 text-center text-xs text-ink-3">{stampText(event.ts)}</p>}
			<Row event={event} />
		</div>
	);
}

/**
 * A search hit: unpin, bring the row to the middle, and light it briefly.
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

function Row({ event }: { event: TranscriptEvent }) {
	switch (event.kind) {
		case "user":
			return (
				<div className="mt-2 flex justify-end">
					<div className="bubble bubble-me">{event.text}</div>
				</div>
			);

		case "agent":
			return (
				<div className="mt-2 flex">
					<div className="bubble bubble-them">
						<Markdown text={event.text} />
					</div>
				</div>
			);

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

		/* Where the agent's working context reset, on the date stamp's own
		 * line. Nothing to click: in a conversation with a colleague, the fact
		 * that they slept is not an event. */
		case "chapter":
			return (
				<p className="py-3 text-center text-xs text-ink-3">
					{stampText(event.ts)}
					{event.title !== undefined && <span className="ml-2 text-ink-2">{event.title}</span>}
				</p>
			);

		/* Permissions, plans, computer frames, hands-to-human and peer threads
		 * belong to phases this window does not have yet. Nothing draws them
		 * rather than something drawing them wrong. */
		default:
			return null;
	}
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
