import { describe, expect, test } from "bun:test";
import { sideTitle } from "../src/links";

// Work.tsx reaches hotkeys.ts, which asks the shell's globals which system it
// is on when it loads; outside the shell there are none.
if (typeof window === "undefined") Object.assign(globalThis, { window: {} });
const { sameWork } = await import("../src/components/Work");

describe("pressing what opened a card", () => {
	test("is the same work only for the same turn", () => {
		expect(sameWork({ personaId: "p", blockId: null }, { personaId: "p", blockId: null })).toBe(true);
		expect(sameWork({ personaId: "p", blockId: "b1" }, { personaId: "p", blockId: "b2" })).toBe(false);
		expect(sameWork({ personaId: "p", blockId: "b1" }, { personaId: "p", blockId: null })).toBe(false);
	});
});

describe("an untitled side thread", () => {
	test("is called a new side thread until its first line names it", () => {
		expect(sideTitle("")).toBe("New side thread");
		expect(sideTitle(undefined)).toBe("New side thread");
		expect(sideTitle("Fix the CI badge")).toBe("Fix the CI badge");
	});
});
