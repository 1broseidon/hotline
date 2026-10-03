import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Attachment, RunningSide, TranscriptEvent } from "../generated/contract";
import { chordKeys } from "../chords";
import { ArrowDownIcon, CloseIcon } from "../icons";
import { carry } from "../serverFiles";
import { type Streaming, useRun, useSide, useTape } from "../tape";
import { Band } from "../ui/Band";
import { wire } from "../wire";
import { Composer } from "./Composer";
import { Markdown } from "./Markdown";
import { runEnding, type Step, StepRows, stepRuns, stepsSummary, subagentState, type SubagentEvent } from "./Transcript";

/**
 * What the card shows: a turn's work, as the pane asks for it (a run by its
 * caption's id, or the turn running now), or a subagent's run by its id.
 */
export type OpenWork =
	| { personaId: string; blockId: string | null }
	| { personaId: string; runId: string; title: string }
	| { personaId: string; sideId: string; title: string };

/** The same work: pressing what opened a card, with it open, closes it. */
export function sameWork(a: OpenWork, b: OpenWork): boolean {
	if ("sideId" in a) return "sideId" in b && a.sideId === b.sideId;
	if ("runId" in a) return "runId" in b && a.runId === b.runId;
	return "blockId" in b && a.blockId === b.blockId;
}

/** Slack under the newest step that still counts as following it. */
const FOLLOW_SLACK = 48;

/**
 * A teammate's work, as a card floating in the window's corner over the
 * conversation: a window onto what it did, not a second transcript. The
 * steps run top to bottom in the instrument voice and the window follows
 * the newest until you scroll back, so running work can be watched and
 * finished work read. A caption opens its own turn; the mark opens the turn
 * running now, and keeps showing it once it has finished. A subagent's line
 * opens its run here too: the task it was handed, what it did, and what it
 * reported.
 */
export function Work(props: {
	open: OpenWork;
	name: string;
	/** A turn is running for this teammate. */
	live: boolean;
	/** Under the composer rather than floating over the conversation. */
	docked?: boolean;
	/** The teammate's live side threads, for whether the open one is working. */
	sides?: RunningSide[];
	onClose(): void;
}) {
	const { open, sides = [], ...rest } = props;
	if ("sideId" in open) return <SideWork key={open.sideId} open={open} working={sides.find((one) => one.sideId === open.sideId)?.working ?? false} {...rest} />;
	return "runId" in open ? <RunWork open={open} {...rest} /> : <TurnWork open={open} {...rest} />;
}

function TurnWork({
	open,
	name,
	live,
	docked = false,
	onClose,
}: {
	open: { personaId: string; blockId: string | null };
	name: string;
	live: boolean;
	docked?: boolean;
	onClose(): void;
}) {
	const { events, streaming } = useTape(open.personaId);
	const runs = stepRuns(events, streaming);
	const latest = runs[runs.length - 1];
	const run = open.blockId === null ? latest : runs.find((one) => one.id === open.blockId);
	const running = live && run !== undefined && run === latest;
	const waiting = open.blockId === null && live && run === undefined;
	const state = running || waiting ? "Working" : run && runEnding(events, run.items) === "stopped" ? "Stopped" : "Done";

	return (
		<WorkCard
			label={`${name}'s work`}
			heading={state}
			working={state === "Working"}
			{...(run !== undefined ? { detail: stepsSummary(run.items) } : {})}
			docked={docked}
			following={run?.id}
			count={run?.items.length ?? 0}
			onClose={onClose}
		>
			{run === undefined ? (
				<p className="work-empty">{waiting ? `${name} is getting started.` : "This turn's steps are no longer on the tape."}</p>
			) : (
				<StepRows items={run.items} settled={!running} />
			)}
		</WorkCard>
	);
}

/**
 * A subagent's run: the task it was handed, then its steps and what it said
 * in order, its last words being the report its teammate got. Nothing here
 * is answerable: a run is told its task once and reports once.
 */
function RunWork({
	open,
	docked = false,
	onClose,
}: {
	open: { personaId: string; runId: string; title: string };
	docked?: boolean;
	onClose(): void;
}) {
	const { events } = useRun(open.runId);
	const marker = events.find((event): event is SubagentEvent => event.kind === "subagent");
	const title = marker?.title ?? open.title;
	const running = marker === undefined || marker.status === "running";
	const task = events.find((event) => event.kind === "user");
	const pieces = runPieces(events.filter((event) => event !== task));

	return (
		<WorkCard
			label={`Subagent: ${title}`}
			heading={title}
			working={running}
			detail={marker !== undefined ? subagentState(marker) : "starting"}
			docked={docked}
			following={open.runId}
			count={events.length}
			onClose={onClose}
		>
			{task !== undefined && task.kind === "user" && <p className="work-task selectable">{task.text}</p>}
			{pieces.map((piece) =>
				piece.kind === "steps" ? (
					<StepRows key={piece.id} items={piece.items} settled={!running} />
				) : piece.kind === "said" ? (
					<div key={piece.id} className="work-said selectable">
						<Markdown text={piece.text} />
					</div>
				) : (
					<p key={piece.id} className="work-notice selectable">
						{piece.text}
					</p>
				),
			)}
			{pieces.length === 0 && running && <p className="work-empty">The subagent is getting started.</p>}
		</WorkCard>
	);
}

/**
 * A side thread: the same teammate in a second conversation, in the card.
 * What the person said, what the teammate said and did, and below it a
 * composer, because unlike a run this one is answerable. Archive ends it;
 * once archived the card is the thread, read-only, with nothing to type into.
 */
function SideWork({
	open,
	name,
	working,
	docked = false,
	onClose,
}: {
	open: { personaId: string; sideId: string; title: string };
	name: string;
	/** A turn of this thread is running, as the roster says. */
	working: boolean;
	docked?: boolean;
	onClose(): void;
}) {
	const { events, streaming, loaded } = useSide(open.sideId);
	const marker = events.find((event): event is Extract<TranscriptEvent, { kind: "side" }> => event.kind === "side");
	const archived = marker?.status === "archived";
	const title = marker?.title ?? open.title;
	const pieces = sidePieces(events, streaming);
	const [refused, setRefused] = useState<string | null>(null);

	const send = (text: string, attachments: Attachment[]) => {
		setRefused(null);
		void carry(attachments)
			.then((carried) => wire.command("side.prompt", { sideId: open.sideId, text, ...(carried.length > 0 ? { attachments: carried } : {}) }))
			.catch((error: unknown) => setRefused(error instanceof Error ? error.message : String(error)));
	};
	const archive = () => {
		setRefused(null);
		void wire.command("side.archive", { sideId: open.sideId }).catch((error: unknown) => setRefused(error instanceof Error ? error.message : String(error)));
	};

	return (
		<WorkCard
			label={`Side thread with ${name}: ${title}`}
			heading={title}
			working={working && !archived}
			detail={archived ? "archived" : working ? "working" : "side thread"}
			docked={docked}
			tall
			following={open.sideId}
			count={events.length + streaming.length}
			onClose={onClose}
			actions={
				!archived && (
					<button type="button" className="control btn-quiet px-2 text-sm" title="Archive this side thread" onClick={archive}>
						Archive
					</button>
				)
			}
			footer={
				archived ? (
					marker?.result !== undefined && <p className="work-notice selectable">{marker.result}</p>
				) : (
					<>
						{refused !== null && <p className="work-notice selectable">{refused}</p>}
						<Composer
							embedded
							personaId={open.sideId}
							name={name}
							state={working ? "thinking" : "ready"}
							replyQuote={null}
							onSend={send}
							onCancel={() => void wire.command("side.cancel", { sideId: open.sideId }).catch(() => undefined)}
							onClearReply={() => undefined}
						/>
					</>
				)
			}
		>
			{pieces.map((piece) =>
				piece.kind === "steps" ? (
					<StepRows key={piece.id} items={piece.items} settled={!working || archived} />
				) : piece.kind === "said" ? (
					<div key={piece.id} className="work-said selectable">
						<Markdown text={piece.text} />
					</div>
				) : piece.kind === "person" ? (
					<p key={piece.id} className="work-task selectable">
						{piece.text}
					</p>
				) : piece.kind === "permission" ? (
					<SideAsk key={piece.id} sideId={open.sideId} event={piece.event} />
				) : (
					<p key={piece.id} className="work-notice selectable">
						{piece.text}
					</p>
				),
			)}
			{pieces.length === 0 && !archived && <p className="work-empty">{loaded ? `${name} is getting started.` : ""}</p>}
		</WorkCard>
	);
}

type PermissionEvent = Extract<TranscriptEvent, { kind: "permission" }>;

/** A permission card raised inside a side thread: answered by side id. */
function SideAsk({ sideId, event }: { sideId: string; event: PermissionEvent }) {
	const [failed, setFailed] = useState<string | null>(null);
	if (event.decision !== undefined) {
		return <p className="work-notice selectable">{`${event.title} · ${event.decidedOptionName ?? event.decision}`}</p>;
	}
	return (
		<div className="work-said selectable">
			<p>{event.title}</p>
			<div className="mt-1.5 flex flex-wrap gap-1.5">
				{event.options.map((option) => (
					<button
						key={option.optionId}
						type="button"
						className="control btn btn-sm"
						onClick={() =>
							void wire
								.command("side.answer_permission", { sideId, requestId: event.requestId, optionId: option.optionId })
								.catch((error: unknown) => setFailed(error instanceof Error ? error.message : String(error)))
						}
					>
						{option.name}
					</button>
				))}
			</div>
			{failed !== null && <p className="work-notice">{failed}</p>}
		</div>
	);
}

/**
 * A side thread's lines: what the person said, the teammate's steps gathered
 * between what it said, its cards and notices, and whatever is still
 * arriving. The thread's own marker and turn ends are the card's business.
 */
export type SidePiece =
	| RunPiece
	| { kind: "person"; id: string; text: string }
	| { kind: "permission"; id: string; event: PermissionEvent };

export function sidePieces(events: TranscriptEvent[], streaming: Streaming[] = []): SidePiece[] {
	const pieces: SidePiece[] = [];
	const rest: TranscriptEvent[] = [];
	const flush = () => {
		pieces.push(...runPieces(rest.splice(0)));
	};
	for (const event of events) {
		if (event.kind === "user") {
			flush();
			if (event.text.trim() !== "") pieces.push({ kind: "person", id: event.id, text: event.text });
		} else if (event.kind === "permission") {
			flush();
			pieces.push({ kind: "permission", id: event.id, event });
		} else {
			rest.push(event);
		}
	}
	flush();
	for (const live of streaming) {
		if (live.kind === "agent" && live.text.trim() !== "") pieces.push({ kind: "said", id: `live:${live.messageId}`, text: live.text });
	}
	return pieces;
}

type RunPiece = { kind: "steps"; id: string; items: Step[] } | { kind: "said" | "notice"; id: string; text: string };

/** A run's lines after its task: steps gathered between what it said. */
export function runPieces(events: TranscriptEvent[]): RunPiece[] {
	const pieces: RunPiece[] = [];
	for (const event of events) {
		if (event.kind === "thought" || event.kind === "tool") {
			const last = pieces[pieces.length - 1];
			if (last?.kind === "steps") last.items.push(event);
			else pieces.push({ kind: "steps", id: event.id, items: [event] });
		} else if (event.kind === "agent" && event.text.trim() !== "") {
			pieces.push({ kind: "said", id: event.id, text: event.text });
		} else if (event.kind === "notice") {
			pieces.push({ kind: "notice", id: event.id, text: event.text });
		}
	}
	return pieces;
}

/** The card itself: its band, and a window that follows the newest until you scroll back. */
function WorkCard({
	label,
	heading,
	working,
	detail,
	docked,
	following: followKey,
	count,
	onClose,
	actions,
	footer,
	tall = false,
	children,
}: {
	label: string;
	heading: string;
	working: boolean;
	detail?: string;
	docked: boolean;
	/** What the window is following: a new one starts at its newest. */
	following: string | undefined;
	count: number;
	onClose(): void;
	/** What sits in the band before Close: a side thread's Archive. */
	actions?: ReactNode;
	/** Under the window, inside the card: a side thread's composer. */
	footer?: ReactNode;
	/** Room for a conversation rather than a glance. */
	tall?: boolean;
	children: ReactNode;
}) {
	const frame = useRef<HTMLDivElement>(null);
	const [following, setFollowing] = useState(true);
	// Every tape change re-renders this, a thought growing word by word
	// included, so following is kept after each one, not only per new step.
	useLayoutEffect(() => {
		const el = frame.current;
		if (el && following) el.scrollTop = el.scrollHeight;
	});
	useEffect(() => setFollowing(true), [followKey]);

	return (
		<aside className={`${docked ? "work-float work-docked" : "work-float"}${tall ? " work-side" : ""}`} aria-label={label}>
			<Band>
				<div className="min-w-0 flex-1 pl-1">
					<h2 className="flex items-center gap-2 truncate text-lg font-semibold">
						{working && <span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />}
						<span className="truncate">{heading}</span>
					</h2>
				</div>
				{detail !== undefined && <span className="instrument shrink-0 pr-1">{detail}</span>}
				{actions}
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			<div className="relative flex min-h-0 flex-1 flex-col">
				<div
					ref={frame}
					className="work-window"
					onScroll={(scroll) => {
						const el = scroll.currentTarget;
						const atEnd = el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_SLACK;
						if (atEnd !== following) setFollowing(atEnd);
					}}
				>
					{children}
				</div>
				{!following && count > 0 && (
					<button
						type="button"
						className="control btn btn-sm work-newest gap-1"
						onClick={() => {
							setFollowing(true);
							frame.current?.scrollTo({ top: frame.current.scrollHeight, behavior: "smooth" });
						}}
					>
						<ArrowDownIcon />
						Newest
					</button>
				)}
			</div>
			{footer}
		</aside>
	);
}
