import { describe, expect, test } from "bun:test";
import type { Aside, TranscriptEvent } from "../src/generated/contract";
import { asideTime, groupNoted, leftHere, levelWord, withAsides } from "../src/asides";

const aside = (id: string, ts: number, extra: Partial<Aside> = {}): Aside => ({
	id,
	ts,
	to: "quill",
	from: { thread: { kind: "side", key: "release" } },
	by: "goldie",
	text: `aside ${id}`,
	level: "note",
	...extra,
});

describe("asides in the receiver's main conversation", () => {
	const said: TranscriptEvent[] = [
		{ kind: "user", id: "u1", ts: 10, text: "Names for the share button?" },
		{ kind: "agent", id: "a1", ts: 30, text: "On it." },
	];

	test("are drawn at their times among the lines, as aside events", () => {
		const shown = withAsides(said, [aside("x", 20)], false);
		expect(shown.map((event) => event.id)).toEqual(["u1", "x", "a1"]);
		expect(shown[1]!.kind).toBe("aside");
	});

	test("leave out what is older than the first line loaded while older lines are on the desk", () => {
		expect(withAsides(said, [aside("old", 5), aside("x", 20)], true).map((event) => event.id)).toEqual(["u1", "x", "a1"]);
		expect(withAsides(said, [aside("old", 5)], false).map((event) => event.id)).toEqual(["old", "u1", "a1"]);
		expect(withAsides(said, [], false)).toBe(said);
	});

	test("are told apart from the ones a thread left, by whom they are for and where they came from", () => {
		const here = { kind: "dm" as const, key: "quill" };
		expect(leftHere(aside("x", 1), "quill", here)).toBe(false);
		// Quill's own thread left Goldie one: on that thread it is Quill's.
		expect(leftHere(aside("y", 1, { to: "goldie" }), "quill", { kind: "side", key: "mine" })).toBe(true);
		// A work thread of Quill's leaving its main conversation a note.
		expect(leftHere(aside("z", 1, { from: { thread: { kind: "side", key: "mine" } } }), "quill", { kind: "side", key: "mine" })).toBe(true);
	});
});

describe("the Noted page's groups", () => {
	test("are what the person has not seen, then the rest, each newest first", () => {
		const { fresh, earlier } = groupNoted([aside("a", 1, { seen: 9 }), aside("b", 3), aside("c", 2), aside("d", 4, { seen: 9 })]);
		expect(fresh.map((one) => one.id)).toEqual(["b", "c"]);
		expect(earlier.map((one) => one.id)).toEqual(["d", "a"]);
	});

	test("keep what was new when the page opened under New once it is marked seen", () => {
		const { fresh, earlier } = groupNoted([aside("a", 1, { seen: 9 }), aside("b", 3, { seen: 9 })], new Set(["b"]));
		expect(fresh.map((one) => one.id)).toEqual(["b"]);
		expect(earlier.map((one) => one.id)).toEqual(["a"]);
	});

	test("say a person aside is to you, and the others by their level", () => {
		expect(levelWord("person")).toBe("to you");
		expect(levelWord("nudge")).toBe("nudge");
		expect(levelWord("note")).toBe("note");
	});
});

test("an aside's time is the clock today, the day this week, else the date", () => {
	const now = new Date(2026, 9, 31, 21, 41).getTime();
	expect(asideTime(new Date(2026, 9, 31, 9, 5).getTime(), now)).toMatch(/9:05/);
	expect(asideTime(new Date(2026, 9, 27, 9, 5).getTime(), now)).toMatch(/^Tue/);
	expect(asideTime(new Date(2026, 8, 3, 9, 5).getTime(), now)).toMatch(/Sep/);
});
