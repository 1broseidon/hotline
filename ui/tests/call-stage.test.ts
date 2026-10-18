import { describe, expect, test } from "bun:test";
import { callWorking } from "../src/components/CallStage";
import type { SessionInfo } from "../src/generated/contract";

describe("whether the teammate on the call is working", () => {
	const row = (id: string, state: "ready" | "thinking" | "starting" | "idle", working = false) => ({
		persona: { id } as never,
		session: { personaId: id, state } as SessionInfo,
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
