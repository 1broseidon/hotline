import { type KeyboardEvent, type MouseEvent, useEffect, useMemo, useState } from "react";
import type { ThreadId, TranscriptEvent } from "../../generated/contract";
import { LinkCard } from "../LinkCard";
import { onlyLink, previewLink } from "../../linkPreview";
import { chordKeys } from "../../chords";
import { CheckIcon, CopyIcon, ReplyIcon, SmileIcon, SpeakerIcon } from "../../icons";
import { popupMessageMenu, writeClipboard } from "../../native";
import { lineOf, type Run } from "../../transcriptFolds";
import { Markdown } from "../Markdown";
import { type BlockCopy, BubbleToolbar } from "../Sheet";
import { SaidBy } from "../../ui/OpenBand";
import { SentFiles, type SentItem } from "../SentFile";

/** A message being answered: the id the wire stamps, the line the chip shows. */
export type ReplyTarget = { eventId: string; text: string };

/** A line to react to: its id, and all of what it said, for the quote the reaction carries. */
export type ReactTarget = { eventId: string; text: string };

/** What the line a reply answers said, and who said it, for the quote over the reply. */
export type Quote = { who: string; said: string };

/**
 * The emoji on a line: the teammate's own (`theirs`, which came on the
 * line itself) and yours (sent as lines of their own, from the phone or this
 * window, and folded onto it).
 */
export type Reacted = { theirs?: string[] | undefined; yours?: string[] | undefined };

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
 * An agent's line, focusable so R can answer it without a pointer. The files
 * it sent sit under its words, or stand alone when they came without any;
 * `files`, given, are a turn's worth sent one message each, drawn as one.
 * A reply to a call is the version written to be read; what was said aloud
 * for it rides above it as a transcript line. A post from another of the
 * teammate's threads names where it came from above it.
 */
export function AgentSay({
	personaId,
	event,
	files,
	run,
	reactions = {},
	bubble,
	onReply,
	onReact,
    onOpenThread,
}: {
	personaId: string;
	/** The colour the person gave this teammate's bubbles; grey when none. */
	bubble?: string | undefined;
    onOpenThread?(open: { thread: ThreadId; title?: string }): void;
	event: Extract<TranscriptEvent, { kind: "agent" }>;
	files?: SentItem[] | undefined;
	run: Run;
	reactions?: Reacted | undefined;
	onReply?(target: ReplyTarget): void;
	onReact?(target: ReactTarget, emoji: string): void;
}) {
	const sent = files ?? (event.attachments ?? []).map((file, index) => ({ eventId: event.id, index, file }));
	const wordless = event.text.trim() === "";
	const reply = () => onReply?.({ eventId: event.id, text: lineOf(event) });
	// One card for the first link written out bare, as on the phone.
	const link = event.attachments?.length ? null : previewLink(event.text);
	const actions: BubbleActionsProps = {
		copy: () => void writeClipboard(event.text.trim() !== "" ? event.text : lineOf(event)),
		...(onReply !== undefined ? { reply } : {}),
		...(onReact !== undefined ? { react: (emoji: string) => onReact({ eventId: event.id, text: event.text || lineOf(event) }, emoji) } : {}),
	};
	const origin = postedFrom(event);
	const said = useMemo(() => ({ personaId, ts: event.ts }), [personaId, event.ts]);
	// One toolbar: a table, code or diagram pointed at adds its keys to it.
	const [slot, setSlot] = useState<HTMLElement | null>(null);
	const [block, setBlock] = useState<{ id: string; level: number; copy?: BlockCopy } | null>(null);
	const tools = useMemo(
		() => ({ node: slot, active: block?.id ?? null, offer: (id: string, level: number, copy?: BlockCopy) => setBlock({ id, level, ...(copy ? { copy } : {}) }) }),
		[slot, block?.id],
	);
	// Back to the message's own keys once the pointer is on its words again,
	// or has left the message: not on leaving the block, so its keys can be
	// reached in the toolbar beside it.
	const onWords = (over: MouseEvent<HTMLElement>) => {
		const at = over.target as HTMLElement;
		if (block !== null && at.closest(".sheet, .bubble-actions") === null) setBlock(null);
	};
	return (
		<SaidBy.Provider value={said}>
		{/* biome-ignore lint/a11y/noStaticElementInteractions: the pointer leaving the message puts its toolbar back; a keyboard reaches each block's keys by focusing it. */}
		<div className={`said-group relative ${run.top ? "mt-1" : "mt-3"}`} onMouseLeave={() => setBlock(null)}>
			{origin !== null && (event.from !== undefined && onOpenThread !== undefined ? <button type="button" className="said-name" onClick={() => onOpenThread({ thread: { kind: event.from!.kind, key: event.from!.thread }, ...(event.from!.title !== undefined ? { title: event.from!.title } : {}) })}>{origin}</button> : <p className="said-name">{origin}</p>)}
			{event.spoken !== undefined && event.spoken.trim() !== "" && <SaidAloud text={event.spoken} />}
			{!wordless && (
				// biome-ignore lint/a11y/noStaticElementInteractions: the pointer back on the words puts the toolbar's own keys back; nothing is done by it.
				<div className={`speech said-them ${runClass(run)}`} data-bubble={bubble} onMouseOver={onWords} {...answerable(actions, onReply === undefined ? undefined : reply)}>
					<BubbleToolbar.Provider value={tools}>
						<Markdown text={event.text} />
					</BubbleToolbar.Provider>
					{sent.length === 0 && <Reactions {...reactions} />}
					<BubbleActions {...actions} slot={setSlot} level={block?.level ?? null} {...(block?.copy ? { block: block.copy } : {})} />
				</div>
			)}
			{sent.length > 0 && (
				<div className="said-files" {...(wordless ? answerable(actions, onReply === undefined ? undefined : reply) : {})}>
					<SentFiles personaId={personaId} items={sent} />
					<Reactions {...reactions} />
					{wordless && <BubbleActions {...actions} />}
				</div>
			)}
			{event.from !== undefined && onOpenThread !== undefined && (event.from.sources?.length ? event.from.sources : event.from.kind === "call" ? [{ kind: event.from.kind, key: event.from.thread }] : []).map((thread, index, sources) => <button key={`${thread.kind}:${thread.key}`} type="button" className="said-name" onClick={() => onOpenThread({ thread })}>{sources.length > 1 ? `Open call ${index + 1}` : "Open the call"}</button>)}
			{link !== null && onlyLink(event.text, link) && <LinkCard url={link} always />}
		</div>
		</SaidBy.Provider>
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
	personaId,
	event,
	quote,
	run,
	reactions = {},
	onJump,
	onReply,
}: {
	personaId: string;
	event: Extract<TranscriptEvent, { kind: "user" }>;
	quote: Quote | undefined;
	run: Run;
	reactions?: Reacted | undefined;
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
	// What you sent rides over your words, as the phone draws it: pictures as
	// themselves, other files as rows. A line of only files has no bubble.
	const files = (event.attachments ?? []).map((file, index) => ({ eventId: event.id, index, file }));
	const wordless = text.trim() === "" && quote === undefined && files.length > 0;
	return (
		<div className={`said-group flex flex-col items-end ${run.top ? "mt-1" : "mt-3"}`}>
			{onCall && !run.top && (
				<span className="said-on-call">
					<SpeakerIcon className="said-aloud-mark" />
					Said on the call
				</span>
			)}
			{files.length > 0 && (
				<div className="own-files">
					<SentFiles personaId={personaId} items={files} mine />
				</div>
			)}
			{!wordless && (
				// biome-ignore lint/a11y/noStaticElementInteractions: the menu is the pointer's way to the copy and reply in the row beside the bubble.
				<div className={`speech said-me ${runClass(run)}`} onContextMenu={(click) => bubbleMenu(click, actions)}>
					{quote !== undefined && answered !== undefined && (
						<button type="button" className="quote" title="Go to the message" onClick={() => onJump(answered)}>
							<span className="quote-who">{quote.who}</span>
							<span className="quote-said">{quote.said}</span>
						</button>
					)}
					{text}
					{event.receipt !== undefined && <Ticks read={event.receipt === "read"} />}
					<Reactions {...reactions} />
					<BubbleActions {...actions} />
				</div>
			)}
			{link !== null && onlyLink(text, link) && <LinkCard url={link} always mine />}
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
/** Where a block in the bubble hands its keys, and how far down it stands. */
type Belt = { slot?: (node: HTMLElement | null) => void; level?: number | null; block?: BlockCopy };

/**
 * What a bubble offers a pointer: react, reply, copy, and after them the
 * keys of the table, code or diagram the pointer is on, one belt that moves
 * level with that block. It sits beside the bubble on its open side and
 * shows on hover or focus, the desktop's long press. The face swaps the row
 * for the six emoji until the pointer leaves.
 */
function BubbleActions({ copy, reply, react, slot, level, block }: BubbleActionsProps & Belt) {
	const [picking, setPicking] = useState(false);
	const [copied, setCopied] = useState(false);
	useEffect(() => {
		if (!copied) return;
		const done = setTimeout(() => setCopied(false), 1200);
		return () => clearTimeout(done);
	}, [copied]);
	return (
		<span
			className="bubble-actions"
			role="toolbar"
			aria-label="Message actions"
			style={level !== null && level !== undefined ? { top: level } : undefined}
			onMouseLeave={() => setPicking(false)}
		>
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
						title={copied ? "Copied" : (block?.title ?? "Copy")}
						aria-label={copied ? "Copied" : (block?.title ?? "Copy")}
						onClick={() => {
							// One Copy: the block pointed at, else the message.
							if (block !== undefined) void writeClipboard(block.text);
							else copy();
							setCopied(true);
						}}
					>
						{copied ? <CheckIcon /> : <CopyIcon />}
					</button>
				</>
			)}
			{slot !== undefined && <span ref={slot} className="bubble-actions-more" hidden={picking} />}
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

/**
 * The keys that answer a line from the keyboard: R replies, and the menu key
 * or a right-click opens its menu. Focusable only where a reply can be sent.
 */
function answerable(actions: BubbleActionsProps, reply: (() => void) | undefined) {
	return {
		tabIndex: reply === undefined ? undefined : 0,
		onContextMenu: (click: MouseEvent<HTMLElement>) => bubbleMenu(click, actions),
		onKeyDown: (key: KeyboardEvent<HTMLElement>) => {
			if (reply === undefined) return;
			if (key.repeat) return;
			if (key.ctrlKey || key.altKey || key.metaKey) return;
			if (key.key !== "r" && key.key !== "R") return;
			key.preventDefault();
			reply();
		},
	};
}

/**
 * The emoji on a line, as chips on its lower edge: one chip an emoji, with
 * how many when more than one, yours ringed in the accent.
 */
function Reactions({ theirs = [], yours = [] }: Reacted) {
	if (theirs.length === 0 && yours.length === 0) return null;
	const chips = new Map<string, { count: number; mine: boolean }>();
	for (const one of theirs) chips.set(one, { count: (chips.get(one)?.count ?? 0) + 1, mine: chips.get(one)?.mine ?? false });
	for (const one of yours) chips.set(one, { count: (chips.get(one)?.count ?? 0) + 1, mine: true });
	return (
		<span className="reactions" aria-label={`Reactions: ${[...theirs, ...yours].join(" ")}`}>
			{[...chips].map(([emoji, { count, mine }]) => (
				<span key={emoji} className="reaction" data-mine={mine || undefined}>
					{emoji}
					{count > 1 && <small>{count}</small>}
				</span>
			))}
		</span>
	);
}
