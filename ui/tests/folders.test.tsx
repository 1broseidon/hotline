import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { Persona, SessionInfo } from "../src/generated/contract";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Teammate } = await import("../src/components/Teammate");
const { wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");

const persona: Persona = {
	id: "ada", name: "Ada", goal: "", backendId: "hotline", cwd: "/work/ada",
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
	folders: [{ path: "/private/tmp/notes", writable: false }],
};
const session: SessionInfo = {
	personaId: "ada", state: "idle", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
	capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
};

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Call = { cmd: string; params: Record<string, unknown> };
let calls: Call[];
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let dom: Window;
let restores: (() => void)[];

beforeEach(async () => {
	dom = new Window();
	restores = [];
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, IS_REACT_ACT_ENVIRONMENT: true, ResizeObserver: NoResize })) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => descriptor ? Object.defineProperty(globalThis, key, descriptor) : Reflect.deleteProperty(globalThis, key));
	}
	// A desk with no endpoint: the room's wire never opens, so the pane reads no settings.
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	calls = [];
	// Everything but the save is refused, which the pane reads as "nothing to show".
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push({ cmd, params });
		if (cmd === "persona.update") return persona;
		throw new Error(`Not in this test: ${cmd}`);
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	replaceDesks([]);
	for (const restore of restores.reverse()) restore();
	await dom.happyDOM.close();
});

async function mount(who: Persona, info: SessionInfo) {
	await act(async () => {
		root.render(
			<Teammate persona={who} session={info} jobs={[]} roster={[]} focusSchedules={false} onClose={() => {}} onDeleted={() => {}} onOpenThread={() => {}} />,
		);
	});
}

describe("a teammate's extra folders", () => {
	test("a folder shows its path and Can edit, and turning it on saves the whole list", async () => {
		await mount(persona, session);
		expect(container.textContent).toContain("Also let it read");
		expect(container.textContent).toContain("/private/tmp/notes");
		expect(container.textContent).toContain("Add folder");
		expect(container.textContent).toContain("Its file tools and protected shell reach these too");
		const canEdit = [...container.querySelectorAll("label")].find((node) => node.textContent === "Can edit")?.querySelector("input");
		if (!canEdit) throw new Error("No Can edit switch");
		expect(canEdit.checked).toBe(false);
		await act(async () => canEdit.click());
		expect(calls.filter(({ cmd }) => cmd === "persona.update")).toEqual([
			{ cmd: "persona.update", params: { id: "ada", patch: { folders: [{ path: "/private/tmp/notes", writable: true }] } } },
		]);
	});

	test("a running harness that takes no extra folders says so", async () => {
		await mount({ ...persona, backendId: "claude" }, { ...session, state: "ready" });
		expect(container.textContent).toContain("It does not take extra folders");
	});
});
