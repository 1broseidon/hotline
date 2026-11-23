import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { NewTeammateForm } = await import("../src/components/NewTeammate");
const { wire } = await import("../src/wire");
const { replaceDesks, wireFor } = await import("../src/desks");

// The agents a desk offers, as the registry names them. A harness runs as its
// own process on the person's account, outside Hotline's sandbox
// (driver/acp/session.rs `spawn`), so the form must not say it is kept to its
// folder; only Hotline Agent's own tools are.
const BACKENDS = [
	{ id: "hotline", name: "Hotline Agent", description: "" },
	{ id: "claude-acp", name: "Claude Code", description: "" },
	{ id: "codex-acp", name: "Codex", description: "" },
	{ id: "cursor", name: "Cursor", description: "" },
];

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;
let defaultBackendId = "hotline";

beforeEach(async () => {
	restores = [];
	({ teardown } = installDom({ ResizeObserver: NoResize }));
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	const connection = wireFor("local");
	const subscription = spyOn(connection, "subscribe").mockImplementation((_target, handlers) => {
		let cancelled = false;
		queueMicrotask(() => {
			if (!cancelled) handlers.snapshot([{ kind: "setting", id: "defaultBackendId", value: defaultBackendId }] as never[]);
		});
		return () => {
			cancelled = true;
		};
	});
	restores.push(() => subscription.mockRestore());
	const command = spyOn(connection, "command").mockImplementation((async (cmd: string) => {
		if (cmd === "backends.list") return BACKENDS;
		if (cmd === "computer.runtimes") return [];
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
	await teardown();
});

async function mount(backendId: string, simple: boolean) {
	defaultBackendId = backendId;
	await act(async () => {
		root.render(<NewTeammateForm models={[]} simple={simple} onCreated={() => {}} />);
	});
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 5));
	});
	// The folder hint is inside its fold: open it.
	const head = [...container.querySelectorAll<HTMLButtonElement>(".nt-fold-head")].find((one) => one.textContent?.startsWith("Folders"));
	if (!head) throw new Error("No Folders fold");
	await act(async () => head.click());
}

const text = () => container.textContent ?? "";

for (const backend of BACKENDS.filter((one) => one.id !== "hotline")) {
	for (const simple of [false, true]) {
		test(`a ${backend.name} teammate is not said to be kept to its folder (${simple ? "welcome" : "full"} form)`, async () => {
			await mount(backend.id, simple);
			expect(text()).not.toContain("Its folder only");
			expect(text()).not.toContain("only touches files here");
			expect(text()).toContain(`${backend.name} runs with its own permissions`);
			if (!simple) expect(text()).toContain(`Managed by ${backend.name}`);
		});
	}
}

for (const simple of [false, true]) {
	test(`a Hotline Agent teammate keeps its folder words (${simple ? "welcome" : "full"} form)`, async () => {
		await mount("hotline", simple);
		expect(text()).toContain("only touches files here");
		if (!simple) expect(text()).toContain("Its folder only");
		expect(text()).not.toContain("Managed by");
	});
}
