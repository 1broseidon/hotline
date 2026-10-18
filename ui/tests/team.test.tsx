import { expect, test } from "bun:test";
import type { RosterEntry } from "../src/wire";
import { shownTeam, teamOrder } from "../src/pins";
import { installDom } from "./dom";

const entry = (id: string, more: { pin?: number; latest?: number; waiting?: boolean } = {}) =>
	({
		persona: { id, name: id.toUpperCase(), goal: "" },
		session: { personaId: id, state: "idle" },
		waiting: more.waiting ?? false,
		drawing: false,
		...(more.latest === undefined ? {} : { latest: more.latest }),
		...(more.pin === undefined ? {} : { pin: more.pin }),
	}) as unknown as RosterEntry;

const ids = (rows: { deskId: string; entry: RosterEntry }[]) => rows.map((row) => `${row.deskId}/${row.entry.persona.id}`);

test("one desk keeps the rail's order exactly", () => {
	const order = teamOrder([{ deskId: "local", entries: [entry("a", { latest: 1 }), entry("b", { latest: 9 }), entry("c", { pin: 0 })] }]);
	expect(ids(order.all)).toEqual(["local/c", "local/a", "local/b"]);
});

test("several desks list each desk's pins, then who waits on the person, then who spoke last", () => {
	const order = teamOrder([
		{ deskId: "local", entries: [entry("clementine", { latest: 10 }), entry("toad", { pin: 0, latest: 1 })] },
		{ deskId: "grizzly", entries: [entry("lou", { latest: 30 }), entry("m", { latest: 5, waiting: true }), entry("wren", { pin: 0 })] },
	]);
	expect(ids(order.pinned)).toEqual(["local/toad", "grizzly/wren"]);
	expect(ids(order.rest)).toEqual(["grizzly/m", "grizzly/lou", "local/clementine"]);
});

test("the filter narrows the team to one desk, and the desk on screen reads its own roster", () => {
	const desks = [{ id: "local" }, { id: "grizzly" }];
	const here = [entry("clementine")];
	const elsewhere = { grizzly: [entry("lou")], local: [entry("stale")] };
	expect(shownTeam(desks, "local", null, here, elsewhere).map((one) => [one.deskId, one.entries.map((e) => e.persona.id)])).toEqual([
		["local", ["clementine"]],
		["grizzly", ["lou"]],
	]);
	expect(shownTeam(desks, "local", "grizzly", here, elsewhere).map((one) => one.deskId)).toEqual(["grizzly"]);
	expect(shownTeam([], "", null, here, {})).toEqual([{ deskId: "", entries: here }]);
});

test("one rail lists every desk's teammates with their desk, dims an unreachable one, and opens a teammate on its own desk", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { Rail } = await import("../src/components/Rail");
	const { replaceDesks, setActiveDesk, setDeskFilter, LOCAL_DESK } = await import("../src/desks");
	const desk = (id: string, name: string, kind: "local" | "remote", state?: "open" | "unreachable") => ({
		id,
		name,
		kind,
		origin: `http://127.0.0.1:1`,
		token: `token-${id}`,
		...(state === undefined ? {} : { state }),
	});
	replaceDesks([desk(LOCAL_DESK, "This computer", "local"), desk("grizzly", "grizzly", "remote", "open"), desk("lab", "lab", "remote", "unreachable")]);
	setActiveDesk(LOCAL_DESK);
	setDeskFilter(null);
	const picked: Array<[string, string | undefined]> = [];
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const render = () =>
		root.render(
			<Rail
				entries={[entry("clementine", { latest: 10 })]}
				elsewhere={{ grizzly: [entry("lou", { latest: 20 })], lab: [entry("wren", { latest: 5 })] }}
				selectedId="clementine"
				seen={{ clementine: 10 }}
				connection="open"
				onSelect={(id, deskId) => picked.push([id, deskId])}
				onEdit={() => {}}
				onDelete={() => {}}
				onPin={() => {}}
				onAddDesk={() => {}}
				search={null}
				deskKey={null}
			/>,
		);
	const labels = () => [...container.querySelectorAll("[data-teammate-row]")].map((row) => row.getAttribute("aria-label"));
	try {
		await act(async () => render());
		expect(labels()).toEqual(["LOU · grizzly, unread", "CLEMENTINE · This computer", "WREN · lab, unread, unreachable"]);
		expect(container.querySelector('[aria-label="WREN · lab, unread, unreachable"]')!.hasAttribute("data-offline")).toBe(true);
		await act(async () => {
			(container.querySelector('[aria-label="LOU · grizzly, unread"]') as unknown as HTMLElement).click();
			(container.querySelector('[aria-label="CLEMENTINE · This computer"]') as unknown as HTMLElement).click();
		});
		expect(picked).toEqual([
			["lou", "grizzly"],
			["clementine", undefined],
		]);
		// Narrowed to one desk, the rail lists that desk's teammates alone and never swaps the window.
		await act(async () => setDeskFilter("grizzly"));
		expect(labels()).toEqual(["LOU · grizzly, unread"]);
		await act(async () => setDeskFilter(null));
		expect(labels()).toHaveLength(3);
	} finally {
		await act(async () => {
			root.unmount();
		});
		setDeskFilter(null);
		replaceDesks([]);
		await teardown();
	}
});
