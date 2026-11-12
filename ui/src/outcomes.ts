import { type Glance, glance } from "./glance";
import type { Step } from "./scheduledRuns";

/*
 * What a turn came to and the path it took, in the teammate's own words.
 *
 * The teammate says what it is on (`activity`) and what it left behind
 * (`outcome`); the desk never makes either up. Under a reply, the outcomes
 * are the line: what went out, what was made or changed, what is still
 * going, what is left for you. In the pane, the path: the steps grouped
 * under the activity each happened during, its words the heading, and each
 * outcome marked where it was said. The phone will keep a copy.
 */

type ActivityLine = Extract<Step, { kind: "activity" }>;
export type OutcomeLine = Extract<Step, { kind: "outcome" }>;

/** How an outcome is marked: failed ✕, partial –, open ● (still going), yours →, done ✓, and a kind the desk does not know plainly. */
export type OutcomeMark = "failed" | "partial" | "open" | "yours" | "done" | "plain";

const KINDS = new Set(["sent", "made", "changed", "open", "yours"]);

export function markOf(outcome: OutcomeLine): OutcomeMark {
	if (outcome.state === "failed") return "failed";
	if (outcome.state === "partial") return "partial";
	if (!KINDS.has(outcome.type)) return "plain";
	if (outcome.type === "open") return outcome.closedAt === undefined ? "open" : "done";
	if (outcome.type === "yours") return "yours";
	return "done";
}

/** An open one that ended with nothing left behind has left the line; the path still says it was there. */
function ended(outcome: OutcomeLine): boolean {
	return outcome.type === "open" && outcome.closedAt !== undefined;
}

/** A turn's outcomes as the line under its reply says them: what failed first, then in the order said. */
export function outcomesOf(items: Step[]): OutcomeLine[] {
	const standing = items.filter((item): item is OutcomeLine => item.kind === "outcome" && !ended(item));
	const failed = standing.filter((outcome) => markOf(outcome) === "failed");
	return [...failed, ...standing.filter((outcome) => markOf(outcome) !== "failed")];
}

/** One thing in the path: a line of steps, or an outcome where it was said. */
export type PathEntry = { kind: "line"; line: Glance } | { kind: "outcome"; outcome: OutcomeLine; ended: boolean };

/** The steps said during one activity, under its words; the steps before any, or after one with no words, have no heading. */
export type PathGroup = { heading: ActivityLine | null; entries: PathEntry[] };

/**
 * A turn's path in groups. Steps are read at a glance a run at a time, the
 * run between two things the teammate said, so a line never spans two
 * headings; thinking with no step after it in its run waits for the next.
 * `running` is whether the turn is at work: its last line says what is
 * happening now.
 */
export function pathGroups(items: Step[], running: boolean): PathGroup[] {
	const groups: PathGroup[] = [{ heading: null, entries: [] }];
	let run: Step[] = [];
	const flush = (last: boolean) => {
		if (!last && !run.some((item) => item.kind === "tool")) return;
		const group = groups[groups.length - 1]!;
		for (const line of glance(run, running && last)) group.entries.push({ kind: "line", line });
		run = [];
	};
	for (const item of items) {
		if (item.kind === "activity") {
			flush(false);
			groups.push({ heading: item.message !== undefined && item.message.trim() !== "" ? item : null, entries: [] });
		} else if (item.kind === "outcome") {
			flush(false);
			groups[groups.length - 1]!.entries.push({ kind: "outcome", outcome: item, ended: ended(item) });
		} else run.push(item);
	}
	flush(true);
	return groups.filter((group) => group.heading !== null || group.entries.length > 0);
}

/** Every line of steps in a path, in order: what the bar and the count read. */
export function pathLines(groups: PathGroup[]): Glance[] {
	return groups.flatMap((group) => group.entries.flatMap((entry) => (entry.kind === "line" ? [entry.line] : [])));
}
