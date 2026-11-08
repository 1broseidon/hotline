import type { RunningSubagent } from "./generated/contract";

/*
 * A working subagent's latest steps, as the thread list's ticker shows them.
 * The roster row carries only the newest (`lastStep`); the window keeps the
 * few before it as each roster row arrives, so the ticker has lines to roll
 * up without the desk sending a history. Nothing here outlives the window,
 * and the runs least lately heard of are let go past a few dozen.
 */

/** How many lines the ticker shows. */
const TICKER_LINES = 3;

/** The most runs remembered at once; the oldest is forgotten first. */
const KEPT_RUNS = 64;

const kept = new Map<string, string[]>();

/** Takes in a roster row's runs: each run's newest step joins its lines when it is new. */
export function noteRuns(runs: readonly RunningSubagent[] | undefined): void {
	for (const run of runs ?? []) {
		const step = run.lastStep;
		if (step === undefined || step === "") continue;
		const lines = kept.get(run.runId) ?? [];
		if (lines[lines.length - 1] === step) continue;
		kept.delete(run.runId);
		kept.set(run.runId, [...lines, step].slice(-TICKER_LINES));
		if (kept.size > KEPT_RUNS) kept.delete(kept.keys().next().value as string);
	}
}

/** A run's latest lines, oldest first, the newest last: at most three. */
export function tickerOf(run: RunningSubagent): string[] {
	noteRuns([run]);
	return kept.get(run.runId) ?? [];
}
