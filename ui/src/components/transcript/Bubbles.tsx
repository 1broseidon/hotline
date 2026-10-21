import { useEffect, useState, type MouseEvent } from "react";
import type { ThreadId, TranscriptEvent } from "../../generated/contract";
import { LinkCard } from "../LinkCard";
import { onlyLink, previewLink } from "../../linkPreview";
import { chordKeys } from "../../chords";
import { CheckIcon, CopyIcon, ReplyIcon, SmileIcon, SpeakerIcon } from "../../icons";
import { popupMessageMenu, writeClipboard } from "../../native";
import { lineOf, type Run } from "../../transcriptFolds";
import { Markdown } from "../Markdown";
import { SentFile } from "../SentFile";

/** A message being answered: the id the wire stamps, the line the chip shows. */
export type ReplyTarget = { eventId: string; text: string };

/** A line to react to: its id, and all of what it said, for the quote the reaction carries. */
export type ReactTarget = { eventId: string; text: string };

function runClass(run: Run): string {
	return `${run.top ? "said-run-top" : ""} ${run.bottom ? "said-run-bottom" : ""}`;
}

/**
 * Where a message the teammate posted from elsewhere came from: its call, a
 * work thread, a scheduled job. The stamp says it; the words do not.
 */
function postedFrom(event: Extract<TranscriptEvent, { kind: "agent" }>): string | null {
	if (event.from?.title && !(event.from.kind === "call" && event.from.title === "Call")) return `From ${event.from.title}`;
	if (event.scheduled !== undefined) return "From a scheduled job";
	switch (event.from?.kind) {
		case "call":
			return "From the call";
		case "side":
			return "From a work thread";
		case "pair":
			return "From a pair thread";
		case "run":
			return "From a subagent";
		case "job":
			return "From a scheduled job";
		default:
			return null;
	}
}

/**
 * An agent's line, focusable so R can answer it without a pointer. A file
 * it sent sits under its words, or is the whole bubble when it came alone.
 * A reply to a call is the version written to be read; what was said aloud
 * for it rides above it as a transcript line. A post from another of the
 * teammate's threads names where it came from above it.
 */
export function AgentSay({
	personaId,
	event,
	run,
	reactions,
	onReply,
	onReact,
    onOpenThread,
}: {
	personaId: string;
    onOpenThread?(open: { thread: ThreadId; title?: string }): void;
	event: Extract<TranscriptEvent, { kind: "agent" }>;
	run: Run;
	reactions: string[] | undefined;
	onReply?(target: ReplyTarget): void;
	onReact?(target: ReactTarget, emoji: string): void;
}) {
	const reply = () => onReply?.({ eventId: event.id, text: lineOf(event) });
	// One card for the first link written out bare, as on the phone.
	const link = event.attachments?.length ? null : previewLink(event.text);
	const actions: BubbleActionsProps = {
		copy: () => void writeClipboard(event.text.trim() !== "" ? event.text : lineOf(event)),
		...(onReply !== undefined ? { reply } : {}),
		...(onReact !== undefined ? { react: (emoji: string) => onReact({ eventId: event.id, text: event.text || lineOf(event) }, emoji) } : {}),
	};
	const origin = postedFrom(event);
	return (
		<div className={`said-group relative ${run.top ? "mt-1" : "mt-3"}`}>
			{origin !== null && (event.from !== undefined && onOpenThread !== undefined ? <button type="button" className="said-name" onClick={() => onOpenThread({ thread: { kind: event.from!.kind, key: event.from!.thread }, ...(event.from!.title !== undefined ? { title: event.from!.title } : {}) })}>{origin}</button> : <p className="said-name">{origin}</p>)}
			{event.spoken !== undefined && event.spoken.trim() !== "" && <SaidAloud text={event.spoken} />}
			{/* biome-ignore lint/a11y/noStaticElementInteractions: the bubble is text; focus is there so R replies and the menu key opens its menu. */}
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
					<SentFile compact={event.from?.kind === "call" && (event.attachments?.length ?? 0) > 3} key={file.path} personaId={personaId} eventId={event.id} index={index} file={file} />
				))}
				<Reactions emoji={reactions} />
				<BubbleActions {...actions} />
			</div>
			{event.from !== undefined && onOpenThread !== undefined && (event.from.sources?.length ? event.from.sources : event.from.kind === "call" ? [{ kind: event.from.kind, key: event.from.thread }] : []).map((thread, index, sources) => <button key={`${thread.kind}:${thread.key}`} type="button" className="said-name" onClick={() => onOpenThread({ thread })}>{sources.length > 1 ? `Open call ${index + 1}` : "Open the call"}</button>)}
			{link !== null && <LinkCard url={link} always={onlyLink(event.text, link)} />}
		</div>
	);
}

/**
 * What the teammate said on the call for the reply under it: one quiet line
 * with a speaker, cut to the column until pressed, then whole. The reply is
 * the version written to be read; this is the version that was heard, kept
 * so the conversation shows both.
 */
export function SaidAloud({ text }: { text: string }) {
	const [open, setOpen] = useState(false);
	return (
		<button type="button" className={`said-aloud ${open ? "said-aloud-open" : ""}`} aria-expanded={open} onClick={() => setOpen((was) => !was)}>
			<SpeakerIcon className="said-aloud-mark" />
			<span className="sr-only">Said on the call: </span>
			<span className="said-aloud-text">{text}</span>
		</button>
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
export function NamedSay({ name, mine, text }: { name: string; mine: boolean; text: string }) {
	if (mine) {
		return (
			<div className="mt-3 flex flex-col items-end">
				<p className="said-name">{name}</p>
				<div className="speech said-me">
					<Markdown text={text} />
				</div>
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

export function UserBubble({
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
	const link = previewLink(text);
	const actions: BubbleActionsProps = {
		copy: () => void writeClipboard(text),
		...(onReply !== undefined && sent ? { reply: () => onReply({ eventId: event.id, text: lineOf({ text }) }) } : {}),
	};
	// Said on a call and handed over: the call's own record has the rest of it.
	const onCall = event.id.startsWith("voice:");
	return (
		<div className={`said-group flex flex-col items-end ${run.top ? "mt-1" : "mt-3"}`}>
			{onCall && !run.top && (
				<span className="said-on-call">
					<SpeakerIcon className="said-aloud-mark" />
					Said on the call
				</span>
			)}
			{/* biome-ignore lint/a11y/noStaticElementInteractions: the menu is the pointer's way to the copy and reply in the row beside the bubble. */}
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
			{link !== null && <LinkCard url={link} always={onlyLink(text, link)} mine />}
		</div>
	);
}

/** The stored text minus a leading quote block the old composer prepended. */
function unquoted(text: string): string {
	const lines = text.split("\n");
	let end = 0;
	while (end < lines.length && lines[end]!.startsWith(">")) end++;
	if (end === 0) return text;
	return lines.slice(end).join("\n").replace(/^\n+/, "");
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
const REACTIONS = ["👍", "❤️", "😂", "🔥", "👀", "🙏"] as const;

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
