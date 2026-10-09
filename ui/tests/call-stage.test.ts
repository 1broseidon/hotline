import { describe, expect, test } from "bun:test";
import { barLength, barsOpacity, callWorking, stageRing } from "../src/components/CallStage";
import type { CallPhase } from "../src/voice/call";

const PHASES: CallPhase[] = ["connecting", "listening", "hearing", "thinking", "speaking", "held", "ended"];

describe("what runs round the call's mark", () => {
	test("bars while the reply speaks, else the arc while the teammate works, else nothing", () => {
		for (const phase of PHASES) {
			expect(stageRing(phase, true)).toBe(phase === "speaking" ? "bars" : "working");
			expect(stageRing(phase, false)).toBe(phase === "speaking" ? "bars" : null);
		}
	});

	test("silence lays every bar flat; the reply lifts them, louder further", () => {
		for (let index = 0; index < 60; index++) {
			for (const seconds of [0, 0.4, 2.3]) {
				expect(barLength(index, 0, seconds)).toBe(2);
				const quiet = barLength(index, 0.3, seconds);
				const loud = barLength(index, 1, seconds);
				expect(quiet).toBeGreaterThanOrEqual(2);
				expect(loud).toBeGreaterThan(quiet);
				// Never past the stage's edge: 168 across, bars from 53 out.
				expect(53 + loud).toBeLessThan(84);
			}
		}
	});

	test("the bars sway: neighbours differ, and a bar moves over time", () => {
		expect(barLength(0, 1, 1)).not.toBeCloseTo(barLength(1, 1, 1));
		expect(barLength(5, 1, 0)).not.toBeCloseTo(barLength(5, 1, 0.1));
	});

	test("the bars are faint at a whisper and whole at a voice", () => {
		expect(barsOpacity(0)).toBeCloseTo(0.45);
		expect(barsOpacity(0.5)).toBeCloseTo(0.45 + 0.55 * 0.7);
		expect(barsOpacity(1)).toBe(1);
	});
});

describe("whether the teammate on the call is working", () => {
	const row = (id: string, state: "ready" | "thinking" | "starting" | "idle", working = false) => ({
		persona: { id } as never,
		session: { personaId: id, state },
		sides: working ? [{ sideId: "s", title: "t", startedAt: 0, working: true }] : [],
	});

	test("a teammate call follows that teammate's turn or work thread only", () => {
		expect(callWorking([row("goldie", "thinking"), row("mack", "ready")], "goldie")).toBe(true);
		expect(callWorking([row("goldie", "starting")], "goldie")).toBe(true);
		expect(callWorking([row("goldie", "ready", true)], "goldie")).toBe(true);
		expect(callWorking([row("goldie", "ready"), row("mack", "thinking")], "goldie")).toBe(false);
		expect(callWorking([], "goldie")).toBe(false);
	});

	test("a desk call counts any teammate's work", () => {
		expect(callWorking([row("goldie", "ready"), row("mack", "thinking")], undefined)).toBe(true);
		expect(callWorking([row("goldie", "idle"), row("mack", "ready")], undefined)).toBe(false);
	});
});
