import { expect, test } from "bun:test";
import type { RosterEntry } from "../src/wire";
import { nudged, railOrder } from "../src/pins";
import { installDom } from "./dom";

const entry = (id: string, pin?: number) =>
	({
		persona: { id, name: id.toUpperCase(), goal: "" },
		session: { personaId: id, state: "idle" },
		waiting: false,
		drawing: false,
		...(pin === undefined ? {} : { pin }),
	}) as unknown as RosterEntry;

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
					onAddDesk={() => {}}
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
