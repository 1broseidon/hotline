import { describe, expect, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { untouched } = await import("../src/components/Starters");

const line = (kind: string, id: string) => ({ kind, id, ts: 1 }) as unknown as TranscriptEvent;

describe("the starter card", () => {
	test("shows on a tape where nothing has been said", () => {
		expect(untouched([], false)).toBe(true);
		expect(untouched([line("chapter", "c1")], false)).toBe(true);
	});

	test("goes once the person, a handoff or the teammate has spoken", () => {
		expect(untouched([line("user", "u1")], false)).toBe(false);
		expect(untouched([line("delivery", "handoff:1")], false)).toBe(false);
		expect(untouched([line("agent", "a1")], false)).toBe(false);
	});

	test("a window opened on a long tape's last lines is never a first conversation", () => {
		expect(untouched([line("tool", "t1"), line("turn", "t2")], true)).toBe(false);
	});
});
