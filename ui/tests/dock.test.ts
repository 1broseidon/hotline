import { describe, expect, test } from "bun:test";
import type { ThreadSummary } from "../src/generated/contract";
import {
	clampDock,
	DOCK_KEY,
	DOCK_MAX,
	DOCK_MIN,
	DOCK_WIDTH,
	dockOverlays,
	draggedDock,
	groupThreads,
	type LinkEvent,
	loadDockWidth,
	openerWords,
	pairWith,
	powersOf,
	relativeTime,
	rowOfLink,
	rowOpener,
	rowTitle,
	saveDockWidth,
	threadDot,
	withLinks,
} from "../src/dock";

const thread = (key: string, kind: ThreadSummary["thread"]["kind"] = "side", extra: Partial<ThreadSummary> = {}): ThreadSummary => ({
	thread: { kind, key },
	personaId: "p",
	title: key,
	state: "live",
	startedAt: 1,
	updatedAt: 1,
	working: false,
	waiting: false,
	...extra,
});

const link = (key: string, extra: Partial<LinkEvent> = {}): LinkEvent => ({
	kind: "link",
	id: `link:side:${key}`,
	ts: 5,
	thread: key,
	threadKind: "side",
	personaId: "p",
	title: key,
	state: "live",
	...extra,
});

const store = (initial: Record<string, string> = {}) => {
	const held = { ...initial };
	return { held, getItem: (key: string) => held[key] ?? null, setItem: (key: string, value: string) => void (held[key] = value) };
};

describe("the right-hand pane's width", () => {
	test("is clamped to its narrowest and widest", () => {
		expect(clampDock(10)).toBe(DOCK_MIN);
		expect(clampDock(10_000)).toBe(DOCK_MAX);
		expect(clampDock(400.4)).toBe(400);
	});

	test("opens at the default with nothing remembered, and with something unreadable", () => {
		expect(loadDockWidth(store())).toBe(DOCK_WIDTH);
		expect(loadDockWidth(store({ [DOCK_KEY]: "{not json" }))).toBe(DOCK_WIDTH);
		expect(loadDockWidth(store({ [DOCK_KEY]: JSON.stringify({ width: "wide" }) }))).toBe(DOCK_WIDTH);
	});

	test("is remembered, clamped, the way it was left", () => {
		const kept = store();
		saveDockWidth(480, kept);
		expect(loadDockWidth(kept)).toBe(480);
		saveDockWidth(9_999, kept);
		expect(loadDockWidth(kept)).toBe(DOCK_MAX);
		expect(loadDockWidth(store({ [DOCK_KEY]: JSON.stringify({ width: 5 }) }))).toBe(DOCK_MIN);
	});

	test("widens as its edge is dragged left, from where the drag began", () => {
		expect(draggedDock(360, 500, 450)).toBe(410);
		expect(draggedDock(360, 500, 600)).toBe(DOCK_MIN);
		expect(draggedDock(360, 500, 0)).toBe(DOCK_MAX);
	});

	test("lies over the conversation only when it would leave it too narrow", () => {
		expect(dockOverlays(1000, 360)).toBe(false);
		expect(dockOverlays(760, 360)).toBe(true);
		expect(dockOverlays(900, 560)).toBe(true);
		expect(dockOverlays(1100, 560)).toBe(false);
	});
});

describe("the list of threads", () => {
	const keys = (rows: ThreadSummary[]) => rows.map((row) => `${row.thread.kind}:${row.thread.key}`);

	test("is work threads first, then pairs, runs and calls, with the closed ones folded apart", () => {
		const { open, closed } = groupThreads([
			thread("c1", "call", { updatedAt: 90 }),
			thread("r1", "run", { updatedAt: 80 }),
			thread("a~b", "pair", { updatedAt: 70 }),
			thread("old", "side", { state: "parked", updatedAt: 10 }),
			thread("done-early", "side", { state: "closed", updatedAt: 20 }),
			thread("new", "side", { updatedAt: 50 }),
			thread("done-late", "run", { state: "closed", updatedAt: 40 }),
		]);
		expect(keys(open)).toEqual(["side:new", "side:old", "pair:a~b", "run:r1", "call:c1"]);
		expect(keys(closed)).toEqual(["run:done-late", "side:done-early"]);
	});

	test("puts what wants you at the head of its kind, however old, and parked last", () => {
		const { open } = groupThreads([
			thread("parked", "side", { state: "parked", updatedAt: 99 }),
			thread("idle", "side", { updatedAt: 90 }),
			thread("running", "side", { working: true, updatedAt: 10 }),
			thread("asks", "side", { waiting: true, updatedAt: 1 }),
		]);
		expect(keys(open)).toEqual(["side:asks", "side:running", "side:idle", "side:parked"]);
	});

	test("leaves out the teammate's own conversation, which is the window beside it", () => {
		expect(groupThreads([thread("p", "dm")])).toEqual({ open: [], closed: [] });
	});

	test("is two empty groups for no threads", () => {
		expect(groupThreads([])).toEqual({ open: [], closed: [] });
	});

	test("says what each is doing: running, waiting on you, parked, closed", () => {
		expect(threadDot(thread("a", "side", { working: true }))).toBe("running");
		expect(threadDot(thread("a"))).toBe("idle");
		expect(threadDot(thread("a", "side", { state: "parked" }))).toBe("parked");
		expect(threadDot(thread("a", "side", { state: "parked", waiting: true }))).toBe("waiting");
		expect(threadDot(thread("a", "side", { state: "closed", working: true }))).toBe("closed");
	});

	test("names a row by its title, else by the teammate on the other side of a pair", () => {
		const names = (id: string) => ({ ada: "Ada", mack: "Mack" })[id];
		expect(rowTitle(thread("x", "side", { title: "Fix CI" }), "ada", names)).toBe("Fix CI");
		const pair = thread("ada~mack", "pair", { personaId: "ada", withPersonaId: "mack" });
		const { title: _gone, ...untitled } = pair;
		expect(rowTitle(untitled, "ada", names)).toBe("With Mack");
		expect(rowTitle(untitled, "mack", names)).toBe("With Ada");
		expect(pairWith("ada~mack", "mack")).toBe("ada");
	});

	test("says whose hands a handoff came from, and the kind of what is not work", () => {
		const handed = thread("h", "side", { opener: { personaId: "mack", name: "Mack" } });
		expect(openerWords(handed)).toBe("from Mack");
		expect(rowOpener(handed)).toBe("from Mack");
		expect(openerWords(thread("m", "side", { opener: { personaId: "p", name: "Ada" } }))).toBe("");
		expect(rowOpener(thread("r", "run"))).toBe("Subagent");
		expect(rowOpener(thread("c", "call"))).toBe("Call");
		expect(rowOpener(thread("s"))).toBe("");
	});
});

describe("a link as a row", () => {
	test("carries the thread's name, state, how it ended and who opened it", () => {
		const row = rowOfLink(link("s1", { threadKind: "run", state: "closed", end: "done", at: 99, outcome: "Found it.", openerId: "mack", openerName: "Mack" }), "p");
		expect(row).toEqual({
			thread: { kind: "run", key: "s1" },
			personaId: "p",
			title: "s1",
			state: "closed",
			end: "done",
			opener: { personaId: "mack", name: "Mack" },
			startedAt: 5,
			updatedAt: 99,
			working: false,
			waiting: false,
			outcome: "Found it.",
		});
	});

	test("falls back to the teammate whose conversation holds it when the link does not say", () => {
		const { personaId: _, ...bare } = link("s1");
		expect(rowOfLink(bare, "ada").personaId).toBe("ada");
	});

	test("brings a listed row up to what the links say since, and adds one the list has not heard of", () => {
		const listed = [thread("a", "side", { working: true, waiting: true, preview: "on it", updatedAt: 10 }), thread("b", "side")];
		const rows = withLinks(listed, [link("a", { state: "closed", end: "done", outcome: "Done.", at: 50 }), link("fresh", { title: "New one" })], "p");
		const byKey = Object.fromEntries(rows.map((row) => [row.thread.key, row]));
		expect(byKey["a"]).toMatchObject({ state: "closed", end: "done", outcome: "Done.", working: false, waiting: false, updatedAt: 50, preview: "on it" });
		expect(byKey["b"]).toEqual(listed[1]!);
		expect(byKey["fresh"]).toMatchObject({ title: "New one", state: "live" });
		expect(rows).toHaveLength(3);
	});

	test("leaves a row alone when the link agrees, and ignores the conversation's own", () => {
		const listed = [thread("a", "side", { working: true })];
		expect(withLinks(listed, [link("a")], "p")).toEqual(listed);
		expect(withLinks([], [link("p", { threadKind: "dm" })], "p")).toEqual([]);
	});
});

describe("what a person can do in a thread", () => {
	test("is speak and archive in an open work thread, and continue in a closed one", () => {
		expect(powersOf("side", "live")).toEqual({ say: true, close: true, resume: false });
		expect(powersOf("side", "parked")).toEqual({ say: true, close: true, resume: false });
		expect(powersOf("side", "closed")).toEqual({ say: false, close: false, resume: true });
	});

	test("is only reading in a pair, a run and a call", () => {
		for (const kind of ["pair", "run", "call"] as const) {
			for (const state of ["live", "parked", "closed"] as const) expect(powersOf(kind, state)).toEqual({ say: false, close: false, resume: false });
		}
	});
});

describe("relative times", () => {
	test("says how long ago in the shortest word", () => {
		const now = 10_000_000_000;
		expect(relativeTime(now - 5_000, now)).toBe("now");
		expect(relativeTime(now - 5 * 60_000, now)).toBe("5m");
		expect(relativeTime(now - 3 * 3_600_000, now)).toBe("3h");
		expect(relativeTime(now - 2 * 86_400_000, now)).toBe("2d");
		expect(relativeTime(now + 5_000, now)).toBe("now");
	});
});
