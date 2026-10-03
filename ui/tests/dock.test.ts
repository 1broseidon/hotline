import { describe, expect, test } from "bun:test";
import type { SideThreadSummary } from "../src/generated/contract";
import {
	clampDock,
	DOCK_KEY,
	DOCK_MAX,
	DOCK_MIN,
	DOCK_WIDTH,
	dockOverlays,
	draggedDock,
	groupSides,
	loadDockWidth,
	relativeTime,
	saveDockWidth,
	sideState,
} from "../src/dock";

const side = (sideId: string, extra: Partial<SideThreadSummary> = {}): SideThreadSummary => ({
	sideId,
	personaId: "p",
	title: sideId,
	status: "live",
	startedAt: 1,
	lastAt: 1,
	working: false,
	waiting: false,
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

describe("the list of side threads", () => {
	test("keeps the open ones first, newest first, and folds the archived apart", () => {
		const { open, archived } = groupSides([
			side("old", { status: "parked", lastAt: 10 }),
			side("done-early", { status: "archived", lastAt: 99, archivedAt: 20 }),
			side("new", { lastAt: 50 }),
			side("done-late", { status: "archived", lastAt: 30, archivedAt: 40 }),
		]);
		expect(open.map((one) => one.sideId)).toEqual(["new", "old"]);
		expect(archived.map((one) => one.sideId)).toEqual(["done-late", "done-early"]);
	});

	test("is two empty groups for no threads", () => {
		expect(groupSides([])).toEqual({ open: [], archived: [] });
	});

	test("says what each is doing: running, waiting on you, parked, archived", () => {
		expect(sideState(side("a", { working: true }))).toBe("running");
		expect(sideState(side("a"))).toBe("idle");
		expect(sideState(side("a", { status: "parked" }))).toBe("parked");
		expect(sideState(side("a", { status: "parked", waiting: true }))).toBe("waiting");
		expect(sideState(side("a", { status: "archived", working: true }))).toBe("archived");
	});

	test("says how long ago in the shortest word", () => {
		const now = 10_000_000_000;
		expect(relativeTime(now - 5_000, now)).toBe("now");
		expect(relativeTime(now - 5 * 60_000, now)).toBe("5m");
		expect(relativeTime(now - 3 * 3_600_000, now)).toBe("3h");
		expect(relativeTime(now - 2 * 86_400_000, now)).toBe("2d");
		expect(relativeTime(now + 5_000, now)).toBe("now");
	});
});
