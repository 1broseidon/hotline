import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { RosterEntry } from "../src/wire";
import { installDom } from "./dom";

/*
 * An idle window asks nothing of the desk. The rail's desk menu shows how a
 * desk's link is doing, and a link moves with no event the window hears, so
 * it is asked for every few seconds; but only the open menu shows it. Asked
 * for with the menu closed, each answer was a React commit every 4 s for as
 * long as the window lived, and in WebKit those commits leave 512 KB
 * regions in the web process that it never gives back.
 */

const entry = (id: string) =>
	({ persona: { id, name: id.toUpperCase(), goal: "" }, session: { personaId: id, state: "idle" }, waiting: false, drawing: false, latest: 1 }) as unknown as RosterEntry;

let teardown: () => Promise<void>;
let ticks: Map<number, () => void>;
let asked: string[];
let answer: unknown[];
let restores: (() => void)[];

beforeEach(async () => {
	({ teardown } = installDom());
	ticks = new Map();
	asked = [];
	answer = [];
	restores = [];
	let next = 1;
	const every = spyOn(globalThis, "setInterval").mockImplementation(((fn: () => void) => {
		const id = next++;
		ticks.set(id, fn);
		return id;
	}) as unknown as typeof setInterval);
	const stop = spyOn(globalThis, "clearInterval").mockImplementation(((id: number) => {
		ticks.delete(id);
	}) as unknown as typeof clearInterval);
	restores.push(() => every.mockRestore(), () => stop.mockRestore());
	const { wireFor, replaceDesks, setActiveDesk, setDeskFilter, LOCAL_DESK } = await import("../src/desks");
	const desk = (id: string, kind: "local" | "remote") => ({ id, name: id === LOCAL_DESK ? "This computer" : "grizzly", kind, origin: "http://127.0.0.1:1", token: `t-${id}`, ...(kind === "remote" ? { state: "open" as const } : {}) });
	replaceDesks([desk(LOCAL_DESK, "local"), desk("grizzly", "remote")]);
	setActiveDesk(LOCAL_DESK);
	setDeskFilter(null);
	const own = wireFor(LOCAL_DESK);
	const command = spyOn(own, "command").mockImplementation((async (cmd: string) => {
		asked.push(cmd);
		if (cmd === "desks.links") return answer.map((one) => ({ ...(one as object) }));
		return null;
	}) as typeof own.command);
	restores.push(() => command.mockRestore());
});

afterEach(async () => {
	const { replaceDesks, setDeskFilter } = await import("../src/desks");
	for (const restore of restores.reverse()) restore();
	setDeskFilter(null);
	replaceDesks([]);
	await teardown();
});

async function mountRail() {
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { Rail } = await import("../src/components/Rail");
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	await act(async () =>
		root.render(
			<Rail
				entries={[entry("clementine")]}
				elsewhere={{ grizzly: [entry("lou")] }}
				selectedId="clementine"
				seen={{ clementine: 1 }}
				connection="open"
				onSelect={() => {}}
				onEdit={() => {}}
				onDelete={() => {}}
				onPin={() => {}}
				onLead={() => {}}
				onAddDesk={() => {}}
				onNew={() => {}}
				settings={null}
				search={null}
				deskKey={null}
			/>,
		),
	);
	return { act, root, container };
}

test("with the desk menu closed the rail asks the desk nothing and sets no timer; open, it asks at once and every few seconds; closed again, it stops", async () => {
	const { act, root, container } = await mountRail();
	try {
		const key = container.querySelector('[aria-label^="Desks: "]') as unknown as HTMLElement;
		expect(key).not.toBeNull();
		expect(asked.filter((cmd) => cmd === "desks.links")).toEqual([]);
		expect(ticks.size).toBe(0);

		answer = [{ deskId: "grizzly", state: "connected" }];
		await act(async () => key.click());
		expect(asked.filter((cmd) => cmd === "desks.links")).toHaveLength(1);
		expect(ticks.size).toBe(1);
		expect(document.body.textContent).toContain("Teammates linked");

		answer = [{ deskId: "grizzly", state: "reconnecting" }];
		await act(async () => {
			for (const tick of ticks.values()) tick();
		});
		expect(asked.filter((cmd) => cmd === "desks.links")).toHaveLength(2);
		expect(document.body.textContent).toContain("Linked, reconnecting…");

		await act(async () => key.click());
		expect(ticks.size).toBe(0);
	} finally {
		await act(async () => root.unmount());
	}
});

test("an answer that says what the last one said draws nothing again", async () => {
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { useDeskLinks } = await import("../src/deskLinks");
	let renders = 0;
	function Reader() {
		renders++;
		const links = useDeskLinks();
		return <span>{links.length}</span>;
	}
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	answer = [{ deskId: "grizzly", state: "connected" }];
	try {
		await act(async () => root.render(<Reader />));
		const settled = renders;
		expect(container.textContent).toBe("1");
		for (let i = 0; i < 5; i++) {
			await act(async () => {
				for (const tick of ticks.values()) tick();
			});
		}
		expect(asked.filter((cmd) => cmd === "desks.links").length).toBeGreaterThanOrEqual(6);
		expect(renders).toBe(settled);
	} finally {
		await act(async () => root.unmount());
	}
});
