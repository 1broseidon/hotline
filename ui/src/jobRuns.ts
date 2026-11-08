import type { ScheduleEntry, TranscriptEvent } from "./generated/contract";
import type { Step } from "./scheduledRuns";

/*
 * A job's thread as its runs (docs/design/thread-panes.html, tab Job). Each
 * firing is a turn on the job's own thread, opened by its scheduled line, and
 * a line the person says in the thread is a turn too; each is one section,
 * from the line that opened it to the turn that ended it.
 */

type JobRunState = "working" | "posted" | "done" | "stopped" | "failed";

export type JobRun = {
	/** The line that opened it. */
	id: string;
	ts: number;
	/** A firing's place among the firings loaded, from 1; none for the person's line. */
	number?: number;
	/** What the person said, when they opened it rather than the schedule. */
	asked?: string;
	state: JobRunState;
	/** What it came to: its last words, after its last step. */
	found: string;
	/** Notices said on the thread while it ran: a skipped firing, a stop. */
	notices: { id: string; text: string }[];
	steps: Step[];
};

/** A step that put something in the person's chat: `post_to_chat`, `send_file`, as the desk's words for them say. */
function posts(step: Step): boolean {
	return step.kind === "tool" && step.status === "completed" && (step.said?.key === "post" || step.said?.key === "send");
}

/**
 * The runs on a job's thread, oldest first. Whether a run posted follows the
 * desk's rule (hotline-core session/job_thread.rs, `LiveJob::end`): a step
 * of its own posted, or it was a firing of a job that is not quiet and it
 * ended with words, not stopped. A run with no turn end yet is working while
 * the job's turn runs, and done when it does not (a desk that went down
 * mid-run never wrote one).
 */
export function jobRuns(events: TranscriptEvent[], working: boolean): JobRun[] {
	type Open = JobRun & { quiet: boolean; firing: boolean; ended: boolean; said: string[] };
	const runs: Open[] = [];
	let firings = 0;
	for (const event of events) {
		if (event.kind === "user") {
			const firing = event.scheduled !== undefined;
			if (firing) firings += 1;
			runs.push({
				id: event.id,
				ts: event.ts,
				...(firing ? { number: firings } : { asked: event.text }),
				state: "working",
				found: "",
				notices: [],
				steps: [],
				quiet: event.scheduled?.quiet === true,
				firing,
				ended: false,
				said: [],
			});
			continue;
		}
		const run = runs[runs.length - 1];
		if (run === undefined) continue;
		switch (event.kind) {
			case "thought":
				run.steps.push(event);
				break;
			case "tool":
				run.steps.push(event);
				// What it said before a step was on the way, not what it found.
				run.said = [];
				break;
			case "agent":
				if (event.text.trim() !== "") run.said.push(event.text.trim());
				break;
			case "notice":
				run.notices.push({ id: event.id, text: event.text });
				if (event.level === "error") run.state = "failed";
				if (event.text.startsWith("Stopped.")) run.state = "stopped";
				break;
			case "turn":
				run.ended = true;
				if (run.state === "working") run.state = event.stopReason === "end_turn" ? "done" : event.stopReason === "cancelled" ? "stopped" : "failed";
				break;
		}
	}
	return runs.map(({ quiet, firing, ended, said, ...run }, at) => {
		const found = said.join("\n\n");
		const last = at === runs.length - 1;
		let state = run.state;
		if (state === "working" && !(last && working && !ended)) state = "done";
		if (state !== "working" && state !== "stopped" && (run.steps.some(posts) || (firing && !quiet && found !== ""))) state = "posted";
		return { ...run, state, found };
	});
}

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

/** A time of day, for a run's line and the last post. */
export function timeOfDay(at: number): string {
	return clock.format(new Date(at));
}

/** How long until the next run, counting down: 4:59, 1:04:59, or in days past one. */
export function countdown(ms: number): string {
	const seconds = Math.max(0, Math.ceil(ms / 1000));
	if (seconds >= 86_400) return `${Math.round(seconds / 86_400)}d`;
	const hours = Math.floor(seconds / 3600);
	const minutes = Math.floor((seconds % 3600) / 60);
	const rest = String(seconds % 60).padStart(2, "0");
	return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}` : `${minutes}:${rest}`;
}

/** An interval as said in a schedule: "5 min", "2 h", "1 day". */
function every(ms: number): string {
	const minutes = Math.round(ms / 60_000);
	if (minutes < 60) return `${minutes} min`;
	const hours = Math.round(minutes / 60);
	if (hours < 24 || hours % 24 !== 0) return `${hours} h`;
	const days = hours / 24;
	return days === 1 ? "1 day" : `${days} days`;
}

/** When a job runs, in a few words: "every 5 min", or "once at 9:00 AM". */
export function scheduleWords(job: ScheduleEntry): string {
	if (job.kind === "loop" && job.every !== undefined) return `every ${every(job.every)}`;
	const at = job.when ?? job.nextAt;
	return `once at ${timeOfDay(at)}`;
}

/** What a job puts in the person's chat: each run's report, or only what it finds worth posting. */
export function postingWords(job: ScheduleEntry): string {
	return job.quiet === true ? "posts only what is worth posting" : "posts each run";
}
