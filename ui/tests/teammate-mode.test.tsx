import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Persona, SessionInfo } from "../src/generated/contract";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Teammate } = await import("../src/components/Teammate");
const { wire } = await import("../src/wire");
const { replaceDesks, wireFor } = await import("../src/desks");

// Goldie runs on Claude Code and is not running: her row carries the modes
// her agent last offered here, her own choice current.
const goldie: Persona = {
	id: "goldie", name: "Goldie", goal: "", backendId: "claude-acp", cwd: "/work/goldie", modeId: "bypassPermissions",
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
};
const resting: SessionInfo = {
	personaId: "goldie", state: "idle", contextRestored: false, models: [], configs: [], slashCommands: [],
	modes: [
		{ id: "default", name: "Manual" },
		{ id: "plan", name: "Plan" },
		{ id: "bypassPermissions", name: "Bypass permissions" },
	],
	currentModeId: "bypassPermissions",
	modeLabel: "Mode",
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
let restores: (() => void)[];
let teardown: () => Promise<void>;

beforeEach(async () => {
	restores = [];
	({ teardown } = installDom({ ResizeObserver: NoResize }));
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	calls = [];
	const connection = wireFor("local");
	const subscription = spyOn(connection, "subscribe").mockImplementation((_target, handlers) => {
		let cancelled = false;
		queueMicrotask(() => { if (!cancelled) handlers.snapshot([]); });
		return () => { cancelled = true; };
	});
	restores.push(() => subscription.mockRestore());
	const command = spyOn(connection, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push({ cmd, params });
		if (cmd === "plan.refresh") return null;
		if (cmd === "session.set_mode") return { ...resting, currentModeId: params["modeId"] };
		if (cmd === "backends.list") return [{ id: "claude-acp", name: "Claude Code", description: "" }];
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

async function mount(info: SessionInfo) {
	await act(async () => {
		root.render(
			<Teammate persona={goldie} session={info} jobs={[]} roster={[]} focusSchedules={false} onClose={() => {}} onDeleted={() => {}} onOpenThread={() => {}} />,
		);
	});
}

const modePicker = () => container.querySelector<HTMLButtonElement>('button[aria-label="Mode"]');

test("a resting harness teammate's remembered modes are chosen from, and a choice is sent", async () => {
	await mount(resting);
	// Opening the pane asks the core to look at the agent when it is due.
	expect(calls.filter((call) => call.cmd === "plan.refresh")).toEqual([{ cmd: "plan.refresh", params: { personaId: "goldie" } }]);
	const picker = modePicker();
	if (!picker) throw new Error("No mode picker");
	expect(picker.title).toBe("Bypass permissions");
	expect(picker.disabled).toBe(false);
	await act(async () => picker.click());
	const plan = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((item) => item.textContent?.includes("Plan"));
	if (!plan) throw new Error("No Plan in the menu");
	await act(async () => plan.click());
	expect(calls.filter((call) => call.cmd === "session.set_mode")).toEqual([
		{ cmd: "session.set_mode", params: { personaId: "goldie", modeId: "plan" } },
	]);
});

test("an agent that has offered no modes here has no mode row", async () => {
	const { currentModeId: _mode, modeLabel: _label, ...plain } = resting;
	await mount({ ...plain, modes: [] });
	expect(modePicker()).toBeNull();
	expect(container.textContent).not.toContain("Runtime mode");
});
