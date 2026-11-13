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

test("the rail shows pinned faces first, and a drop on a face asks for that slot", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { Rail } = await import("../src/components/Rail");
	const pins: Array<[string, number | null]> = [];
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => {
			root.render(
				<Rail
					entries={[entry("a"), entry("b", 1), entry("c", 0)]}
					selectedId={null}
					seen={{}}
					connection="open"
					onSelect={() => {}}
					onEdit={() => {}}
					onDelete={() => {}}
					onPin={(id, slot) => pins.push([id, slot])}
					onArrange={() => {
						throw new Error("a pin's drag does not arrange the list");
					}}
					onLead={() => {}}
					onAddDesk={() => {}}
					onNew={() => {}}
					settings={null}
					search={null}
					deskKey={null}
				/>,
			);
		});
		const rows = [...container.querySelectorAll("[data-teammate-row]")];
		expect(rows.map((row) => row.getAttribute("aria-label"))).toEqual(["C, pinned", "B, pinned", "A"]);
		expect(container.querySelectorAll("[data-pin]")).toHaveLength(2);
		const first = container.querySelector('[data-pin="0"]')!;
		const dragged = container.querySelector('[data-pin="1"]')!;
		const transfer = new Map<string, string>();
		const data = { setData: (type: string, value: string) => transfer.set(type, value), getData: (type: string) => transfer.get(type) ?? "", effectAllowed: "", dropEffect: "" };
		const fire = async (target: Element, type: string) =>
			act(async () => {
				const event = new window.Event(type, { bubbles: true, cancelable: true });
				Object.defineProperty(event, "dataTransfer", { value: data });
				target.dispatchEvent(event);
			});
		await fire(dragged, "dragstart");
		await fire(first, "dragover");
		await fire(first, "drop");
		expect(pins).toEqual([["b", 0]]);
	} finally {
		await act(async () => { root.unmount(); });
		await teardown();
	}
});

async function rail(entries: RosterEntry[], onArrange: (ids: string[]) => void, run: (container: HTMLElement, act: typeof import("react").act) => Promise<void>) {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { Rail } = await import("../src/components/Rail");
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
					onSelect={() => {}}
					onEdit={() => {}}
					onDelete={() => {}}
					onPin={() => {
						throw new Error("a row's move does not pin");
					}}
					onArrange={onArrange}
					onLead={() => {}}
					onAddDesk={() => {}}
					onNew={() => {}}
					settings={null}
					search={null}
					deskKey={null}
				/>,
			);
		});
		await run(container, act);
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

test("a row's menu moves it one place up or down its desk's list, and offers neither past its end", async () => {
	const arranged: string[][] = [];
	// Unarranged, the list is the roster's order, and the first move fixes it.
	await rail([entry("a"), entry("b"), entry("c"), entry("p", 0)], (order) => arranged.push(order), async (container, act) => {
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
	});
	expect(arranged).toEqual([
		["b", "a", "c"],
		["b", "a", "c"],
		["a", "c", "b"],
		["a", "c", "b"],
	]);
});

test("a row dropped on another row takes its place in the list, and a pinned face dropped on a row does nothing", async () => {
	const arranged: string[][] = [];
	await rail([placed("a", 0), placed("b", 1), placed("c", 2), entry("p", 0)], (order) => arranged.push(order), async (container, act) => {
		expect([...container.querySelectorAll("[data-teammate-row]")].map((row) => row.getAttribute("aria-label"))).toEqual(["P, pinned", "A", "B", "C"]);
		const transfer = new Map<string, string>();
		const data = { setData: (type: string, value: string) => transfer.set(type, value), getData: (type: string) => transfer.get(type) ?? "", effectAllowed: "", dropEffect: "" };
		const fire = async (target: Element, type: string) =>
			act(async () => {
				const event = new window.Event(type, { bubbles: true, cancelable: true });
				Object.defineProperty(event, "dataTransfer", { value: data });
				target.dispatchEvent(event);
			});
		await fire(rowOf(container, "C"), "dragstart");
		expect(rowOf(container, "C").hasAttribute("data-dragging")).toBe(true);
		await fire(rowOf(container, "A"), "dragover");
		expect(rowOf(container, "A").hasAttribute("data-over")).toBe(true);
		await fire(rowOf(container, "A"), "drop");
		expect(rowOf(container, "C").hasAttribute("data-dragging")).toBe(false);

		await fire(rowOf(container, "P"), "dragstart");
		await fire(rowOf(container, "B"), "dragover");
		expect(rowOf(container, "B").hasAttribute("data-over")).toBe(false);
		await fire(rowOf(container, "B"), "drop");
		await fire(rowOf(container, "P"), "dragend");
	});
	expect(arranged).toEqual([["c", "a", "b"]]);
});
