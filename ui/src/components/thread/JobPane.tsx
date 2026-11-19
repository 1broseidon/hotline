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
import { ErrorCard } from "../ErrorCard";
import { Markdown } from "../Markdown";
import type { ThreadRef } from "../Transcript";
import { TrailSteps, useHot } from "../transcript/Trail";
import { BandKind, PaneHead } from "./Pane";
import { ThreadComposer } from "./ThreadComposer";
import { Mark, type MarkKind, SectionRule, StateLine, waitsOnPerson } from "./Spine";

/*
 * A job's pane (docs/design/thread-panes.html, tab Job): work the person
 * watches, on a schedule. Its head says what it is and how it stands, then
 * its schedule, when it runs next, counted down, and what it last posted;
 * then its Runs, newest first, each a line on the pane's spine that opens to
 * what it found and its steps, the one running open and live. The band has Stop while a run is running and End job while the job
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
	// A last run that sent a colleague something waits for the reply with no run to come: the state line says on whom.
	const waitingOn = !ended && !working && job?.awaiting !== undefined ? job.awaiting.join(", ") : undefined;
	return (
		<section className="dock-page" aria-label={`Job of ${name}: ${title}`}>
			<Band>
				<button type="button" className="control btn-icon pane-back" title="Back" aria-label="Back" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				<BandKind kind={threadFamily("job")} state={ended ? "ended" : working ? "running" : "scheduled"} />
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
				title={title}
				line={<StateLine word={ended ? "Ended" : working ? "Running now" : waitingOn !== undefined ? `Waiting on ${waitingOn}` : "Scheduled"} time={working && runs[0] !== undefined ? timeOfDay(runs[0].ts) : undefined} />}
				facts={[
					// When it runs and what it posts, in one fact: a fourth row would be one too many.
					["Schedule", job === undefined ? schedule : `${schedule} · ${postingWords(job)}`, "wraps"],
					["Next run", ended || job === undefined || waitingOn !== undefined ? "None" : working ? "Running" : countdown(job.nextAt - now)],
					["Last posted", job?.lastPost === undefined ? "Nothing yet" : `${timeOfDay(job.lastPost.at)} · ${firstLine(job.lastPost.text)}`],
				]}
			/>
			{without !== "" && <p className="dock-note selectable">{without}</p>}
			<SectionRule title="Runs" count={runs.length} status={working ? (waitsOnPerson(events, runs[0]?.ts ?? 0) ? "blocked" : "live") : ended ? "stopped" : undefined} />
			<div className="pane-stream">
				<ol className="job-runs spine-list" aria-label="Runs">
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

/** A run's mark on the spine: posted is done, a run that posted nothing or was stopped is dim. */
const RUN_MARKS: Record<JobRun["state"], MarkKind> = { working: "live", posted: "done", done: "stopped", stopped: "stopped", failed: "failed" };

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
				<Mark kind={RUN_MARKS[run.state]} />
				<span className="min-w-0 truncate">
					{runName(run, numbered)} · {STATE_WORDS[run.state]}
				</span>
				<span className="job-run-when">{live ? "now" : timeOfDay(run.ts)}</span>
				{hasBody && (open ? <ChevronDownIcon className="spine-chevron" /> : <ChevronRightIcon className="spine-chevron" />)}
			</button>
			{open && hasBody && (
				<div className="job-run-body">
					{run.found !== "" && (
						<div className="job-run-found selectable">
							<Markdown text={run.found} />
						</div>
					)}
					{run.notices.map((notice) =>
						// A failure reads as the conversation reads it: its title and plain words, the report behind Details.
						notice.failed ? (
							<ErrorCard key={notice.id} text={notice.text} />
						) : (
							<p key={notice.id} className="job-run-notice selectable">
								{notice.text}
							</p>
						),
					)}
					{run.steps.length > 0 && (
						<div className="spine-nested">
							<TrailSteps items={run.steps} settled={!live} hot={hot} />
						</div>
					)}
				</div>
			)}
		</li>
	);
}
