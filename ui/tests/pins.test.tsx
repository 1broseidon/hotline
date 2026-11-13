import { expect, mock, test } from "bun:test";
import type { RosterEntry } from "../src/wire";
import { movedTo, nudged, railOrder, teamOrder } from "../src/pins";
import { installDom } from "./dom";

// The shell's teammate menu, kept to be read and pressed by the test.
type TeammateMenu = { move?: { onUp?(): void; onDown?(): void } };
let menu: TeammateMenu | null = null;
const native = await import("../src/native");
mock.module("../src/native", () => ({
	...native,
	popupTeammateMenu: async (actions: TeammateMenu) => {
		menu = actions;
	},
}));

const entry = (id: string, pin?: number, more: { place?: number; waiting?: boolean; latest?: number } = {}) =>
	({
		persona: { id, name: id.toUpperCase(), goal: "" },
		session: { personaId: id, state: "idle" },
		waiting: more.waiting ?? false,
		drawing: false,
		...(pin === undefined ? {} : { pin }),
		...(more.place === undefined ? {} : { place: more.place }),
		...(more.latest === undefined ? {} : { latest: more.latest }),
	}) as unknown as RosterEntry;
const placed = (id: string, place: number, more: { waiting?: boolean; latest?: number } = {}) => entry(id, undefined, { ...more, place });
const names = (rows: RosterEntry[]) => rows.map((one) => one.persona.id);
const ids = (rows: { deskId: string; entry: RosterEntry }[]) => rows.map((row) => `${row.deskId}/${row.entry.persona.id}`);

test("pins lead in their slots and leave the list, which counts on after them", () => {
	const order = railOrder([entry("a"), entry("b", 1), entry("c"), entry("d", 0)]);
	expect(order.pinned.map((one) => one.persona.id)).toEqual(["d", "b"]);
	expect(order.rest.map((one) => one.persona.id)).toEqual(["a", "c"]);
	expect(order.all.map((one) => one.persona.id)).toEqual(["d", "b", "a", "c"]);
});

test("a row with no pin reads the same as before pins existed", () => {
	const order = railOrder([entry("a"), entry("b")]);
	expect(order.pinned).toEqual([]);
	expect(order.all.map((one) => one.persona.id)).toEqual(["a", "b"]);
});

test("a desk nobody has arranged lists its team as it always has", () => {
	// One desk: the roster's order, whoever waits or spoke last.
	const one = teamOrder([{ deskId: "local", entries: [entry("a", undefined, { latest: 1 }), entry("b", undefined, { latest: 9, waiting: true }), entry("c")] }]);
	expect(ids(one.rest)).toEqual(["local/a", "local/b", "local/c"]);
	// Several: who waits, then who spoke last, across the desks.
	const several = teamOrder([
		{ deskId: "local", entries: [entry("a", undefined, { latest: 1 }), entry("b", undefined, { latest: 9 })] },
		{ deskId: "grizzly", entries: [entry("c", undefined, { latest: 5 }), entry("d", undefined, { latest: 2, waiting: true })] },
	]);
	expect(ids(several.rest)).toEqual(["grizzly/d", "local/b", "grizzly/c", "local/a"]);
});

test("an arranged desk keeps its order when a teammate waits on the person, and lists the unplaced after", () => {
	const order = railOrder([entry("new"), placed("c", 2), placed("a", 0), placed("b", 1, { waiting: true }), entry("p", 0, { place: 3 })]);
	expect(names(order.pinned)).toEqual(["p"]);
	expect(names(order.rest)).toEqual(["a", "b", "c", "new"]);
	expect(names(order.all)).toEqual(["p", "a", "b", "c", "new"]);
});

test("several desks keep each arranged desk's own order and interleave the rest as before", () => {
	const order = teamOrder([
		// Arranged: z before y, though y waits and spoke last.
		{ deskId: "local", entries: [placed("y", 1, { latest: 50, waiting: true }), placed("z", 0, { latest: 1 })] },
		{ deskId: "grizzly", entries: [entry("m", undefined, { latest: 10 }), entry("n", undefined, { latest: 30 })] },
	]);
	expect(ids(order.rest)).toEqual(["grizzly/n", "grizzly/m", "local/z", "local/y"]);
	// Each desk's rows stay in its own order whatever the other desk's are.
	const local = order.rest.filter((row) => row.deskId === "local");
	expect(ids(local)).toEqual(["local/z", "local/y"]);
});

test("a move lifts the one teammate out and puts it in at the index", () => {
	expect(movedTo(["a", "b", "c", "d"], "d", 1)).toEqual(["a", "d", "b", "c"]);
	expect(movedTo(["a", "b", "c", "d"], "a", 2)).toEqual(["b", "c", "a", "d"]);
	expect(movedTo(["a", "b"], "a", 9)).toEqual(["b", "a"]);
});

test("a nudge stops at either end of the row", () => {
	expect(nudged(0, -1, 3)).toBeNull();
	expect(nudged(0, 1, 3)).toBe(1);
	expect(nudged(2, 1, 3)).toBeNull();
	expect(nudged(2, -1, 3)).toBe(1);
});

type Calls = { arranged: string[][]; pinned: Array<[string, number | null]>; selected: string[] };

/** The rail on `entries`, with what it asks of the window kept in `calls`. */
async function rail(entries: RosterEntry[], run: (container: HTMLElement, act: typeof import("react").act, calls: Calls) => Promise<void>) {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { Rail } = await import("../src/components/Rail");
	const calls: Calls = { arranged: [], pinned: [], selected: [] };
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => {
			root.render(
				<Rail
					entries={entries}
					selectedId={null}
					seen={{}}
					connection="open"
					onSelect={(id) => calls.selected.push(id)}
					onEdit={() => {}}
					onDelete={() => {}}
					onPin={(id, slot) => calls.pinned.push([id, slot])}
					onArrange={(order) => calls.arranged.push(order)}
					onLead={() => {}}
					onAddDesk={() => {}}
					onNew={() => {}}
					settings={null}
					search={null}
					deskKey={null}
				/>,
			);
		});
		await run(container, act, calls);
	} finally {
		await act(async () => { root.unmount(); });
		await teardown();
	}
}

const rowOf = (container: HTMLElement, name: string) => {
	const found = [...container.querySelectorAll<HTMLElement>("[data-teammate-row]")].find((node) => node.getAttribute("aria-label")?.split(",")[0] === name);
	if (found === undefined) throw new Error(`no rail row for ${name}`);
	return found;
};

/**
 * A pointer pressed on `from`, moved over each of `over` in turn (the page
 * hit-testing to that element), then let go or not. The rail reads only
 * which element is under the pointer, so each step is a few pixels further.
 */
async function pointer(container: HTMLElement, act: typeof import("react").act, from: string, over: string[], end: "up" | "escape" | "none" = "up") {
	let under = rowOf(container, from);
	document.elementFromPoint = () => under as never;
	const source = rowOf(container, from);
	source.setPointerCapture = () => {};
	const fire = (target: Element, type: string, x: number) =>
		act(async () => {
			target.dispatchEvent(new window.PointerEvent(type, { bubbles: true, cancelable: true, button: 0, pointerId: 1, clientX: x, clientY: x }) as never);
		});
	await fire(source, "pointerdown", 0);
	for (const [step, name] of over.entries()) {
		under = rowOf(container, name);
		await fire(source, "pointermove", 10 * (step + 1));
	}
	if (end === "escape") {
		await act(async () => {
			window.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape" }) as never);
		});
	}
	if (end !== "none") {
		await fire(source, "pointerup", 10 * over.length);
		await act(async () => {
			source.click();
		});
	}
}

test("the rail shows pinned faces first, and a face dragged onto another asks for that slot", async () => {
	await rail([entry("a"), entry("b", 1), entry("c", 0)], async (container, act, calls) => {
		const rows = [...container.querySelectorAll("[data-teammate-row]")];
		expect(rows.map((row) => row.getAttribute("aria-label"))).toEqual(["C, pinned", "B, pinned", "A"]);
		expect(container.querySelectorAll("[data-pin]")).toHaveLength(2);
		await pointer(container, act, "B", ["C"], "none");
		expect(rowOf(container, "B").hasAttribute("data-dragging")).toBe(true);
		expect(rowOf(container, "C").getAttribute("data-over")).toBe("before");
		await pointer(container, act, "B", ["C"]);
		expect(calls.pinned).toEqual([["b", 0]]);
		// The click a drag ends with opens nothing.
		expect(calls.selected).toEqual([]);
		expect(calls.arranged).toEqual([]);
	});
});

test("a row's menu moves it one place up or down its desk's list, and offers neither past its end", async () => {
	// Unarranged, the list is the roster's order, and the first move fixes it.
	await rail([entry("a"), entry("b"), entry("c"), entry("p", 0)], async (container, act, calls) => {
		const rightClick = async (name: string) => {
			menu = null;
			await act(async () => {
				rowOf(container, name).dispatchEvent(new window.MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
			});
			return menu!.move ?? {};
		};
		const first = await rightClick("A");
		expect(first.onUp).toBeUndefined();
		first.onDown!();
		const middle = await rightClick("B");
		middle.onUp!();
		middle.onDown!();
		const last = await rightClick("C");
		expect(last.onDown).toBeUndefined();
		last.onUp!();
		// A pinned face moves left and right, not up and down.
		expect((await rightClick("P")).onUp).toBeUndefined();
		expect(calls.arranged).toEqual([
			["b", "a", "c"],
			["b", "a", "c"],
			["a", "c", "b"],
			["a", "c", "b"],
		]);
	});
});

test("a row dragged onto another takes its place, with a line where it would land", async () => {
	await rail([placed("a", 0), placed("b", 1), placed("c", 2), entry("p", 0)], async (container, act, calls) => {
		expect([...container.querySelectorAll("[data-teammate-row]")].map((row) => row.getAttribute("aria-label"))).toEqual(["P, pinned", "A", "B", "C"]);
		await pointer(container, act, "C", ["B", "A"], "none");
		expect(rowOf(container, "C").hasAttribute("data-dragging")).toBe(true);
		expect(rowOf(container, "A").getAttribute("data-over")).toBe("before");
		expect(rowOf(container, "B").hasAttribute("data-over")).toBe(false);
		await act(async () => {
			rowOf(container, "C").dispatchEvent(new window.PointerEvent("pointerup", { bubbles: true, button: 0, pointerId: 1 }) as never);
		});
		expect(rowOf(container, "C").hasAttribute("data-dragging")).toBe(false);
		await pointer(container, act, "A", ["B"]);
		expect(rowOf(container, "B").hasAttribute("data-over")).toBe(false);
		expect(calls.arranged).toEqual([
			["c", "a", "b"],
			["b", "a", "c"],
		]);
		expect(calls.selected).toEqual([]);
	});
});

test("a press that does not move opens the teammate, Escape drops a drag where it was, and a face dropped on a row does nothing", async () => {
	await rail([placed("a", 0), placed("b", 1), entry("p", 0)], async (container, act, calls) => {
		await pointer(container, act, "A", []);
		expect(calls.selected).toEqual(["a"]);
		await pointer(container, act, "B", ["A"], "escape");
		expect(rowOf(container, "B").hasAttribute("data-dragging")).toBe(false);
		await pointer(container, act, "P", ["A"], "none");
		expect(rowOf(container, "A").hasAttribute("data-over")).toBe(false);
		await pointer(container, act, "P", ["A"]);
		expect(calls.arranged).toEqual([]);
		expect(calls.pinned).toEqual([]);
		expect(calls.selected).toEqual(["a"]);
	});
});
