import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { chordKeys } from "../chords";
import { ArrowDownIcon, CloseIcon } from "../icons";
import { dmOf, useThread } from "../tape";
import { Band } from "../ui/Band";
import { runEnding, StepRows, stepRuns, stepsSummary } from "./Transcript";

/** What the card shows: a turn's work, by its caption's id, or the turn running now. */
export type OpenWork = { personaId: string; blockId: string | null };

/** The same work: pressing what opened a card, with it open, closes it. */
export function sameWork(a: OpenWork, b: OpenWork): boolean {
	return a.blockId === b.blockId;
}

/** Slack under the newest step that still counts as following it. */
const FOLLOW_SLACK = 48;

/**
 * A teammate's current turn, as a card floating in the window's corner over
 * the conversation: a window onto what it did, not a second transcript. The
 * steps run top to bottom in the instrument voice and the window follows
 * the newest until you scroll back, so running work can be watched and
 * finished work read. A caption opens its own turn; the mark opens the turn
 * running now, and keeps showing it once it has finished. A subagent's run, a
 * side thread or a call is a thread, and opens in the right-hand pane.
 */
export function Work({
	open,
	name,
	live,
	docked = false,
	onClose,
}: {
	open: OpenWork;
	name: string;
	/** A turn is running for this teammate. */
	live: boolean;
	/** Under the composer rather than floating over the conversation. */
	docked?: boolean;
	onClose(): void;
}) {
	const { events, streaming } = useThread(dmOf(open.personaId));
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
			<FollowWindow following={followKey} count={count}>
				{children}
			</FollowWindow>
		</aside>
	);
}

/**
 * The window a card reads through: it follows the newest line until you
 * scroll back, and offers a way down again.
 */
function FollowWindow({
	following: followKey,
	count,
	children,
}: {
	/** What the window is following: a new one starts at its newest. */
	following: string | undefined;
	count: number;
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
	);
}
