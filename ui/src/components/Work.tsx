import { useEffect, useRef, useState } from "react";
import { chordKeys } from "../chords";
import { ArrowDownIcon, CloseIcon } from "../icons";
import { useTape } from "../tape";
import { Band } from "../ui/Band";
import { runEnding, StepRows, stepRuns, stepsSummary } from "./Transcript";

/** A turn's work, as the pane asks for it: a run by its caption's id, or the turn running now. */
export type OpenWork = { personaId: string; blockId: string | null };

/** Slack under the newest step that still counts as following it. */
const FOLLOW_SLACK = 48;

/**
 * A teammate's work for one turn, in the inspector's place beside the
 * conversation: a window onto what it did, not a second transcript. The
 * steps run top to bottom in the instrument voice and the window follows
 * the newest until you scroll back, so a running turn can be watched and a
 * finished one read. A caption opens its own run; the mark opens the turn
 * running now, and keeps showing it once it has finished.
 */
export function Work({
	open,
	name,
	live,
	onClose,
}: {
	open: OpenWork;
	name: string;
	/** A turn is running for this teammate. */
	live: boolean;
	onClose(): void;
}) {
	const { events, streaming } = useTape(open.personaId);
	const runs = stepRuns(events, streaming);
	const latest = runs[runs.length - 1];
	const run = open.blockId === null ? latest : runs.find((one) => one.id === open.blockId);
	const running = live && run !== undefined && run === latest;
	const waiting = open.blockId === null && live && run === undefined;
	const state = running || waiting ? "Working" : run && runEnding(events, run.items) === "stopped" ? "Stopped" : "Done";

	const frame = useRef<HTMLDivElement>(null);
	const [following, setFollowing] = useState(true);
	const count = run?.items.length ?? 0;
	useEffect(() => {
		const el = frame.current;
		if (el && following) el.scrollTop = el.scrollHeight;
	}, [count, following, run?.id]);
	useEffect(() => setFollowing(true), [run?.id]);

	return (
		<aside className="inspector work-pane" aria-label={`${name}'s work`}>
			<Band>
				<div className="min-w-0 flex-1 pl-1">
					<h2 className="flex items-center gap-2 truncate text-lg font-semibold">
						{state === "Working" && <span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />}
						{state}
					</h2>
				</div>
				{run !== undefined && <span className="instrument shrink-0 pr-1">{stepsSummary(run.items)}</span>}
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
					{run === undefined ? (
						<p className="work-empty">{waiting ? `${name} is getting started.` : "This turn's steps are no longer on the tape."}</p>
					) : (
						<StepRows items={run.items} />
					)}
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
