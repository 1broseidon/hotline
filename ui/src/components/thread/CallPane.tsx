import { type ReactNode, useEffect, useMemo, useState } from "react";
import type { ThreadId, ThreadSummary, TranscriptEvent } from "../../generated/contract";
import { glance } from "../../glance";
import { ArrowLeftIcon, ChevronDownIcon, ChevronRightIcon } from "../../icons";
import { powersOf } from "../../dock";
import { useRoomRead } from "../../roomReads";
import type { Block, Step } from "../../scheduledRuns";
import { type Streaming, useThread } from "../../tape";
import { startedWords, threadFamily } from "../../threadKinds";
import { heardOnce, toBlocks, withStreaming } from "../../transcriptFolds";
import { Band } from "../../ui/Band";
import { PaneClose } from "../../ui/PaneClose";
import { liveName } from "../../useFor";
import { startCall, useCall, useCallSnapshot } from "../../voice/call";
import { wire } from "../../wire";
import { Markdown } from "../Markdown";
import { SentFiles } from "../SentFile";
import type { ThreadRef } from "../Transcript";
import { TrailSteps, useHot } from "../transcript/Trail";
import { BandKind, LiveStream, PaneHead } from "./Pane";
import { ThreadComposer } from "./ThreadComposer";

/*
 * A call's pane (docs/design/thread-panes.html, tab Call): a thread the
 * person is in, so it stays a conversation, read as a transcript. Each line
 * is labelled by who said it, You or the teammate, in a column of its own,
 * and the work the teammate went off to do between two lines is folded
 * where it fell, one line saying what it checked and how many steps, that
 * opens to them. The band says whether the call is live, how long it has
 * run or ran, and the voice model, with End call while it is live.
 */

/** How long a call has run, as a clock: 12:48, or 1:02:09 past the hour. */
export function callClock(ms: number): string {
	const seconds = Math.max(0, Math.floor(ms / 1000));
	const hours = Math.floor(seconds / 3600);
	const minutes = Math.floor((seconds % 3600) / 60);
	const rest = String(seconds % 60).padStart(2, "0");
	return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}` : `${minutes}:${rest}`;
}

/** The call's own line, which its thread keeps at its head: when it started, whether it stands, and when it ended. */
function linkOf(events: TranscriptEvent[], key: string) {
	return events.findLast((event): event is Extract<TranscriptEvent, { kind: "link" }> => event.kind === "link" && event.threadKind === "call" && event.thread === key);
}

/** The call open in the pane, as the desk keeps it and as this window is on it. */
export function CallPane({
	open,
	row,
	teammate,
	working,
	onChanged,
	onOpen,
	onBack,
	onClose,
}: {
	open: ThreadRef;
	row: ThreadSummary | undefined;
	teammate: { id: string; name: string };
	/** A turn of the call is running: see `turnRunning`. */
	working: boolean;
	onChanged(): void;
	onOpen(open: ThreadRef): void;
	onBack(): void;
	onClose(): void;
}) {
	const id = open.thread;
	const { events, streaming, more, earlier, prompt, interrupt } = useThread(id);
	const link = useMemo(() => linkOf(events, id.key), [events, id.key]);
	const state = link?.state ?? row?.state ?? "live";
	// This window is on the call when the call it holds is this thread's.
	const call = useCall();
	const phase = useCallSnapshot(call).phase;
	const here = call !== null && phase !== "ended" && (call.id === id.key || (call.target?.thread?.kind === "call" && call.target.thread.key === id.key));
	const hungUp = useMemo(() => hangUpOf(events), [events]);
	// The thread outlives the voice on it (it is parked, and can be spoken in by text), so the call is live
	// while this window is on it, or while its thread is live and no hang-up has been said since anyone spoke.
	const live = here || (state === "live" && hungUp === undefined);
	const startedAt = link?.ts ?? row?.startedAt ?? events[0]?.ts;
	const endedAt = live ? undefined : (hungUp ?? link?.at ?? row?.updatedAt);
	// The voice the desk puts on its calls: a live model by its name, else Turn by turn.
	const status = useRoomRead("voice.status", "", live);
	const model = !live || status === undefined ? undefined : status.front !== undefined ? liveName(status.front.providerId) : "Turn by turn";
	const placedOn = events.find((event): event is Extract<TranscriptEvent, { kind: "user" }> => event.kind === "user" && event.client !== undefined)?.client;
	const [refused, setRefused] = useState<string | null>(null);
	const refuse = (error: unknown) => setRefused(error instanceof Error ? error.message : String(error));
	const end = () => {
		setRefused(null);
		if (here && call !== null) {
			call.hangUp();
			onChanged();
			return;
		}
		void wire.command("voice.call_end", { callId: id.key }).then(onChanged, refuse);
	};
	return (
		<CallPage
			thread={id}
			title={row?.title ?? link?.title ?? open.title ?? "Call"}
			personaId={teammate.id}
			name={teammate.name}
			from={placedOn === "phone" ? "from the phone" : placedOn === "desktop" ? "from the desktop" : undefined}
			live={live}
			startedAt={startedAt}
			endedAt={endedAt}
			model={model}
			events={events}
			streaming={streaming}
			working={working && live}
			more={more}
			onEarlier={earlier}
			onEnd={end}
			{...(live && !here ? { onCall: () => void startCall(undefined, { personaId: teammate.id, name: teammate.name, thread: id }).catch(refuse) } : {})}
			onOpenThread={onOpen}
			onBack={onBack}
			onClose={onClose}
			refused={refused}
			open={powersOf("call", state).say}
			composer={powersOf("call", state).say ? <ThreadComposer thread={id} name={teammate.name} working={working} prompt={prompt} interrupt={interrupt} /> : null}
		/>
	);
}

/**
 * When the voice last went off the call: the desk says "Voice disconnected"
 * on the thread when a call on it ends (hotline-core session/calls.rs,
 * `call_ended`), and the wire has no other word for it that a window not on
 * the call can read. A line the person said after it is the voice back on.
 */
export function hangUpOf(events: TranscriptEvent[]): number | undefined {
	for (let at = events.length - 1; at >= 0; at--) {
		const event = events[at]!;
		if (event.kind === "user") return undefined;
		if (event.kind === "notice" && event.text.startsWith("Voice disconnected")) return event.ts;
	}
	return undefined;
}

/** The call as drawn: the band, the head, the transcript and the composer the caller hands in. */
export function CallPage({
	thread,
	title,
	personaId,
	name,
	from,
	live,
	open = live,
	startedAt,
	endedAt,
	model,
	events,
	streaming = [],
	working,
	more = false,
	onEarlier,
	onEnd,
	onCall,
	onOpenThread,
	onBack,
	onClose,
	refused = null,
	composer = null,
	now: fixedNow,
}: {
	thread: ThreadId;
	title: string;
	personaId: string;
	name: string;
	/** Which of the person's apps the call was placed from, in words. */
	from?: string | undefined;
	/** The voice is on the call. */
	live: boolean;
	/** The thread can be spoken in: it outlives the voice until it is closed. */
	open?: boolean;
	startedAt: number | undefined;
	endedAt?: number | undefined;
	/** The voice model the call runs on, when it is known. */
	model?: string | undefined;
	events: TranscriptEvent[];
	streaming?: Streaming[];
	/** The teammate is at work on the call's behalf. */
	working: boolean;
	more?: boolean;
	onEarlier?(): Promise<void>;
	onEnd(): void;
	/** Takes the call on this computer: given while it is live elsewhere. */
	onCall?(): void;
	onOpenThread?(open: ThreadRef): void;
	onBack(): void;
	onClose(): void;
	refused?: string | null;
	composer?: ReactNode;
	/** The time to count from, for a test; the clock otherwise. */
	now?: number;
}) {
	const now = useNow(live && fixedNow === undefined);
	const at = fixedNow ?? now;
	const length = startedAt === undefined ? undefined : callClock((live ? at : (endedAt ?? at)) - startedAt);
	const written = useMemo(() => toBlocks(heardOnce(events.filter((event) => event.kind !== "link"), true), true), [events]);
	const blocks = useMemo(() => withStreaming(written, streaming), [written, streaming]);
	return (
		<section className="dock-page" aria-label={`Call with ${name}: ${title}`}>
			<Band>
				<button type="button" className="control btn-icon -ml-1" title="Back" aria-label="Back" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				<BandKind kind={threadFamily("call")} live={live} state={live ? "live" : "ended"} tone="quiet" />
				{(length !== undefined || model !== undefined) && <span className="instrument shrink-0 text-ink-3">{[length, model].filter(Boolean).join(" · ")}</span>}
				{onCall !== undefined && (
					<button type="button" className="control btn-quiet px-2 text-sm" title="Take this call here" onClick={onCall}>
						Call
					</button>
				)}
				{live && (
					<button type="button" className="control btn-quiet px-2 text-sm" title="End this call" onClick={onEnd}>
						End call
					</button>
				)}
				<PaneClose onClose={onClose} />
			</Band>
			<PaneHead kind={threadFamily("call")} kindInBand title={title} meta={[from, name, startedAt !== undefined && startedWords(startedAt)]} />
			<LiveStream following={`${thread.kind}:${thread.key}`} count={blocks.length} content={blocks} live={working}>
				{more && onEarlier !== undefined && (
					<button type="button" className="control btn btn-sm mb-3 self-center" onClick={() => void onEarlier()}>
						Earlier lines
					</button>
				)}
				<ol className="call-transcript" aria-label="Transcript">
					{blocks.map((block) => (
						<CallRow key={block.kind === "event" ? block.event.id : block.id} block={block} personaId={personaId} name={name} live={working} onOpenThread={onOpenThread} />
					))}
				</ol>
				{blocks.length === 0 && <p className="dock-note">Nothing said yet.</p>}
			</LiveStream>
			{refused !== null && (
				<p className="dock-note selectable" style={{ color: "var(--warn)" }}>
					{refused}
				</p>
			)}
			{open && composer}
		</section>
	);
}

/** Now, each second while `running`. */
function useNow(running: boolean): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		if (!running) return;
		const tick = setInterval(() => setNow(Date.now()), 1000);
		return () => clearInterval(tick);
	}, [running]);
	return now;
}

type Card = Extract<TranscriptEvent, { kind: "permission" | "human_action" | "passkey_ask" | "credential_ask" }>;

/** One row of the transcript: a line said, the work between two lines, a card's way home, or a notice. */
function CallRow({
	block,
	personaId,
	name,
	live,
	onOpenThread,
}: {
	block: Block;
	personaId: string;
	name: string;
	live: boolean;
	onOpenThread?: ((open: ThreadRef) => void) | undefined;
}) {
	if (block.kind === "steps") return <li><CallWork items={block.items} live={live && block.open === true} /></li>;
	if (block.kind !== "event") return null;
	const event = block.event;
	switch (event.kind) {
		case "user":
			return (
				<CallLine who="You" mine>
					<p className="selectable">{event.text}</p>
					{(event.attachments?.length ?? 0) > 0 && <SentFiles personaId={personaId} mine items={(event.attachments ?? []).map((file, index) => ({ eventId: event.id, index, file }))} />}
				</CallLine>
			);
		case "agent": {
			const heard = event.spoken?.trim() ? event.spoken : event.text;
			return (
				<CallLine who={name}>
					{heard.trim() !== "" && <p className="selectable">{heard}</p>}
					{(event.attachments?.length ?? 0) > 0 && <SentFiles personaId={personaId} items={(event.attachments ?? []).map((file, index) => ({ eventId: event.id, index, file }))} />}
					{heard !== event.text && event.text.trim() !== "" && (
						<details className="call-written">
							<summary>Written reply</summary>
							<div className="selectable">
								<Markdown text={event.text} />
							</div>
						</details>
					)}
				</CallLine>
			);
		}
		case "permission":
		case "human_action":
		case "passkey_ask":
		case "credential_ask":
			return <li><CardHome event={event} personaId={personaId} onOpenThread={onOpenThread} /></li>;
		case "notice":
			return (
				<li className="rule-line rule-line-plain" style={{ color: event.level === "info" ? "var(--ink-3)" : event.level === "warn" ? "var(--warn)" : "var(--danger)" }}>
					<span className="selectable font-normal">{event.text}</span>
				</li>
			);
		case "turn":
			return event.stopReason === "end_turn" ? null : <li className="instrument text-right text-ink-4">{event.stopReason.replace(/_/g, " ")}</li>;
		default:
			return null;
	}
}

/** A line said on the call: who said it in the left column, the words in the right. */
function CallLine({ who, mine = false, children }: { who: string; mine?: boolean; children: ReactNode }) {
	return (
		<li className="call-line" data-mine={mine || undefined}>
			<span className="call-who">{who}</span>
			<div className="call-words">{children}</div>
		</li>
	);
}

/** A card the call raised is answered in the home chat: one line saying how it stands, that opens it there. */
function CardHome({ event, personaId, onOpenThread }: { event: Card; personaId: string; onOpenThread?: ((open: ThreadRef) => void) | undefined }) {
	const settled = event.kind === "permission" ? event.decision !== undefined : event.status !== "pending";
	const label = event.kind === "permission" ? event.title : event.kind === "human_action" ? event.reason : event.name;
	return (
		<button type="button" className="rule-line rule-line-plain" onClick={() => onOpenThread?.({ thread: { kind: "dm", key: personaId }, eventId: event.id })}>
			{settled ? "Settled" : "Needs you"}: {label} · Open card in home chat
		</button>
	);
}

/**
 * The work the teammate did between two lines, folded where it fell: what
 * it did at a glance and how many steps, opening to the steps. While it is
 * still at work the line says what it is doing now.
 */
function CallWork({ items, live }: { items: Step[]; live: boolean }) {
	const [open, setOpen] = useState(false);
	const hot = useHot();
	const lines = useMemo(() => glance(items, live), [items, live]);
	const steps = items.filter((item) => item.kind === "tool").length;
	if (lines.length === 0) return null;
	const what = live ? lines[lines.length - 1]!.text : lines.map((line) => line.text).join(", ");
	return (
		<div className="call-work">
			<button type="button" className="call-work-head" aria-expanded={open} onClick={() => setOpen((was) => !was)}>
				<span aria-hidden="true" className={`call-work-dot ${live ? "beat" : ""}`} data-live={live || undefined} />
				<span className="min-w-0 truncate">{what}</span>
				{steps > 0 && <span className="shrink-0 text-ink-3">· {steps === 1 ? "1 step" : `${steps} steps`}</span>}
				{open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />}
			</button>
			{open && <TrailSteps items={items} settled={!live} hot={hot} />}
		</div>
	);
}
