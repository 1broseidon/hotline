import { expect, test } from "bun:test";
import { isWorking, shownState, workingState } from "../src/activity";

test("one reading of working: starting and a turn count, a turn held open only for subagents does not", () => {
	const reads = (state: string, awaitingSubagents?: boolean) => isWorking({ state, awaitingSubagents } as never);
	expect(reads("starting")).toBe(true);
	expect(reads("thinking")).toBe(true);
	expect(reads("thinking", true)).toBe(false);
	for (const state of ["idle", "ready", "error", "stopped"]) expect(reads(state)).toBe(false);
});

test("the thread pane's composer takes the same word the session's does", () => {
	expect(shownState({ state: workingState(true) })).toBe("thinking");
	expect(isWorking({ state: workingState(true) })).toBe(true);
	expect(isWorking({ state: workingState(false) })).toBe(false);
});
