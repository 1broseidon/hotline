import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { TranscriptEvent } from "../generated/contract";
import { chordKeys } from "../chords";
import { ArrowDownIcon, CloseIcon } from "../icons";
import { useRun, useTape } from "../tape";
import { Band } from "../ui/Band";
import { Markdown } from "./Markdown";
import { runEnding, type Step, StepRows, stepRuns, stepsSummary, subagentState, type SubagentEvent } from "./Transcript";

/**
 * What the card shows: a turn's work, as the pane asks for it (a run by its
 * caption's id, or the turn running now), or a subagent's run by its id.
 */
export type OpenWork = { personaId: string; blockId: string | null } | { personaId: string; runId: string; title: string };

/** The same work: pressing what opened a card, with it open, closes it. */
export function sameWork(a: OpenWork, b: OpenWork): boolean {
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
	onClose(): void;
}) {
	const { open, ...rest } = props;
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
		<aside className={docked ? "work-float work-docked" : "work-float"} aria-label={label}>
			<Band>
				<div className="min-w-0 flex-1 pl-1">
					<h2 className="flex items-center gap-2 truncate text-lg font-semibold">
						{working && <span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />}
						<span className="truncate">{heading}</span>
					</h2>
				</div>
				{detail !== undefined && <span className="instrument shrink-0 pr-1">{detail}</span>}
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
		</aside>
	);
}
