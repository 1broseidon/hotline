import { type ReactNode, useEffect, useMemo, useState } from "react";
import type { ScheduleEntry, ThreadSummary, TranscriptEvent } from "../../generated/contract";
import { ArrowLeftIcon, ChevronDownIcon, ChevronRightIcon } from "../../icons";
import { type JobRun, countdown, jobRuns, postingWords, scheduleWords, timeOfDay } from "../../jobRuns";
import { firstLine } from "../../room";
import { useThread } from "../../tape";
import { wire } from "../../wire";
import { threadFamily } from "../../threadKinds";
import { Band } from "../../ui/Band";
import { PaneClose } from "../../ui/PaneClose";
import { powersOf, withoutWords } from "../../dock";
import { Markdown } from "../Markdown";
import type { ThreadRef } from "../Transcript";
import { TrailSteps, useHot } from "../transcript/Trail";
import { BandKind, PaneHead } from "./Pane";
import { ThreadComposer } from "./ThreadComposer";

/*
 * A job's pane (docs/design/thread-panes.html, tab Job): work the person
 * watches, on a schedule. Its head says Loop or Once, what it is, whose it
 * is, when it runs and what it posts; then when it runs next, counted down,
 * and what it last posted; then each run, newest first, as a section of its
 * own that opens to what it found and its steps, the one running open and
 * live. The band has Stop while a run is running and End job while the job
 * lives, and Remove once it has ended, which takes it off the list. There is no Run now: the desk has no command that fires a job
 * early, and a key that does nothing is not drawn.
 */

/** The job open in the pane, as the desk keeps its thread and its schedule. */
export function JobPane({
	open,
	row,
	job,
	teammate,
	working,
	onChanged,
	onBack,
	onClose,
}: {
	open: ThreadRef;
	row: ThreadSummary | undefined;
	/** The job, as the schedules view says; none once it is deleted. */
	job: ScheduleEntry | undefined;
	teammate: { id: string; name: string };
	/** A turn of the job's thread is running: see `turnRunning`. */
	working: boolean;
	onChanged(): void;
	onBack(): void;
	onClose(): void;
}) {
	const id = open.thread;
	const { events, more, earlier, prompt, cancel, interrupt, close } = useThread(id);
	// A job ends on the schedules view before the list is read again.
	const ended = job?.endedAt !== undefined || row?.state === "closed";
	const [refused, setRefused] = useState<string | null>(null);
	const act = (what: () => Promise<unknown>) => {
		setRefused(null);
		void what().then(onChanged, (error: unknown) => setRefused(error instanceof Error ? error.message : String(error)));
	};
	return (
		<JobPage
			title={job?.name ?? row?.title ?? open.title ?? "Job"}
			name={teammate.name}
			job={job}
			ended={ended}
			events={events}
			working={working && !ended}
			more={more}
			onEarlier={earlier}
			onStop={() => act(cancel)}
			onEnd={() => act(close)}
			onRemove={() => act(() => wire.command("schedule.cancel", { id: id.key }).then(onBack))}
			onBack={onBack}
			onClose={onClose}
			refused={refused}
			composer={!powersOf("job", ended ? "closed" : "live").say ? null : <ThreadComposer thread={id} name={teammate.name} working={working} prompt={prompt} interrupt={interrupt} />}
		/>
	);
}

/** The job as drawn: the band, the head, its next run and last post, its runs, and the composer the caller hands in. */
export function JobPage({
	title,
	name,
	job,
	ended,
	events,
	working,
	more = false,
	onEarlier,
	onStop,
	onEnd,
	onRemove,
	onBack,
	onClose,
	refused = null,
	composer = null,
	now: fixedNow,
}: {
	title: string;
	name: string;
	job: ScheduleEntry | undefined;
	ended: boolean;
	events: TranscriptEvent[];
	/** A run is running now. */
	working: boolean;
	/** Older lines are on the desk: the runs are counted from the first loaded, so they go unnumbered. */
	more?: boolean;
	onEarlier?(): Promise<void>;
	onStop(): void;
	onEnd(): void;
	/** Takes an ended job off the list; its thread goes with it. */
	onRemove?(): void;
	onBack(): void;
	onClose(): void;
	refused?: string | null;
	composer?: ReactNode;
	/** The time to count down from, for a test; the clock otherwise. */
	now?: number;
}) {
	const ticking = useNow(!ended && fixedNow === undefined);
	const now = fixedNow ?? ticking;
	const runs = useMemo(() => jobRuns(events, working).reverse(), [events, working]);
	const schedule = job === undefined ? undefined : scheduleWords(job);
	const without = withoutWords(job?.without);
	return (
		<section className="dock-page" aria-label={`Job of ${name}: ${title}`}>
			<Band>
				<button type="button" className="control btn-icon -ml-1" title="Back" aria-label="Back" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				<BandKind kind={threadFamily("job")} live={working} state={ended ? "ended" : working ? "running" : "scheduled"} tone={ended ? "quiet" : "done"} />
				{schedule !== undefined && <span className="instrument shrink-0 text-ink-3">{schedule}</span>}
				{working && (
					<button type="button" className="control btn-quiet px-2 text-sm" title="Stop this run" onClick={onStop}>
						Stop
					</button>
				)}
				{!ended && (
					<button type="button" className="control btn-quiet px-2 text-sm" title="End this job" onClick={onEnd}>
						End job
					</button>
				)}
				{ended && job !== undefined && onRemove !== undefined && (
					<button type="button" className="control btn-quiet px-2 text-sm" title="Take it off the list" onClick={onRemove}>
						Remove
					</button>
				)}
				<PaneClose onClose={onClose} />
			</Band>
			<PaneHead
				kind={threadFamily("job")}
				family={job === undefined ? undefined : job.kind === "loop" ? "Loop" : "Once"}
				title={title}
				meta={[name, schedule, job !== undefined && postingWords(job)]}
			/>
			{without !== "" && <p className="dock-note selectable">{without}</p>}
			<div className="pane-stream">
				<dl className="job-facts">
					<div>
						<dt>Next run</dt>
						<dd className="tabular-nums">{ended || job === undefined ? "None" : working ? "Running" : countdown(job.nextAt - now)}</dd>
					</div>
					<div>
						<dt>Last posted</dt>
						<dd className="truncate">{job?.lastPost === undefined ? "Nothing yet" : `${timeOfDay(job.lastPost.at)} · ${firstLine(job.lastPost.text)}`}</dd>
					</div>
				</dl>
				<ol className="job-runs" aria-label="Runs">
					{runs.map((run) => (
						<RunSection key={run.id} run={run} numbered={!more} />
					))}
				</ol>
				{runs.length === 0 && <p className="dock-note px-0">No runs yet.</p>}
				{more && onEarlier !== undefined && (
					<button type="button" className="control btn btn-sm mt-2 self-center" onClick={() => void onEarlier()}>
						Earlier runs
					</button>
				)}
			</div>
			{refused !== null && (
				<p className="dock-note selectable" style={{ color: "var(--warn)" }}>
					{refused}
				</p>
			)}
			{!ended && composer}
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

const STATE_WORDS: Record<JobRun["state"], string> = {
	working: "working",
	posted: "posted",
	done: "nothing posted",
	stopped: "stopped",
	failed: "failed",
};

/** What a run is called on its line: its number among the firings, or the person's line that opened it. */
function runName(run: JobRun, numbered: boolean): string {
	if (run.asked !== undefined) return `Message · ${firstLine(run.asked)}`;
	return numbered && run.number !== undefined ? `Run ${run.number}` : "Run";
}

/**
 * One run as a section: its line says which run, how it stands and when,
 * and opens to what it found and its steps. The run working now opens on
 * arrival and shows its steps as they come.
 */
function RunSection({ run, numbered }: { run: JobRun; numbered: boolean }) {
	const live = run.state === "working";
	const [open, setOpen] = useState(live);
	const hot = useHot();
	const hasBody = run.found !== "" || run.steps.length > 0 || run.notices.length > 0;
	return (
		<li className="job-run" data-state={run.state} data-open={open || undefined}>
			<button type="button" className="job-run-head" aria-expanded={hasBody ? open : undefined} disabled={!hasBody} onClick={() => setOpen((was) => !was)}>
				<span aria-hidden="true" className={`job-run-dot ${live ? "beat" : ""}`} data-state={run.state} />
				<span className="min-w-0 truncate">
					{runName(run, numbered)} · {STATE_WORDS[run.state]}
				</span>
				<span className="job-run-when">{live ? "now" : timeOfDay(run.ts)}</span>
				{hasBody && (open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />)}
			</button>
			{open && hasBody && (
				<div className="job-run-body">
					{run.found !== "" && (
						<div className="job-run-found selectable">
							<Markdown text={run.found} />
						</div>
					)}
					{run.notices.map((notice) => (
						<p key={notice.id} className="job-run-notice selectable">
							{notice.text}
						</p>
					))}
					{run.steps.length > 0 && <TrailSteps items={run.steps} settled={!live} hot={hot} />}
				</div>
			)}
		</li>
	);
}
