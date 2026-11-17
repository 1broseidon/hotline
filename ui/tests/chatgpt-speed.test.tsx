import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { ConfigChoice, SessionConfig } from "../src/generated/contract";
import type { RosterEntry } from "../src/wire";
import { installDom } from "./dom";

// Fast mode for Hotline Agent on a ChatGPT sign-in, in the teammate's menu:
// the switch for a model whose listing gives it Fast alone, Standard, Fast
// and Ultrafast for one that lists Ultrafast too, nothing for one with
// neither, the same at rest as running, all saying the plan goes faster.

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }) } });
const { SessionPickers } = await import("../src/components/Pickers");
const { replaceDesks, setActiveDesk, wireFor } = await import("../src/desks");

class FakeCore {
	static readonly OPEN = 1;
	static all: FakeCore[] = [];
	readyState = 1;
	sent: Record<string, unknown>[] = [];
	onopen: (() => void) | null = null;
	onmessage: ((message: { data: string }) => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	constructor(readonly url: string) {
		FakeCore.all.push(this);
		queueMicrotask(() => this.onopen?.());
	}
	reply(frame: Record<string, unknown>) {
		this.onmessage?.({ data: JSON.stringify(frame) });
	}
	send(data: string) {
		const frame = JSON.parse(data) as Record<string, unknown>;
		this.sent.push(frame);
		queueMicrotask(() => {
			if ("sub" in frame) this.reply({ sub: frame["id"], snapshot: [] });
			if (!("cmd" in frame)) return;
			const answers: Record<string, unknown> = {
				"client.hello": { capabilities: ["threads2"] },
				"models.efforts": { choices: [{ id: "high", name: "High" }], defaultId: "high" },
				"backends.list": [],
			};
			this.reply({ id: frame["id"], ok: true, result: answers[String(frame["cmd"])] ?? null });
		});
	}
	/** What the menu asked the core to set, as `config=value`. */
	settings() {
		return this.sent
			.filter((frame) => frame["cmd"] === "session.set_config")
			.map((frame) => {
				const args = frame["params"] as Record<string, unknown>;
				return `${String(args["configId"])}=${String(args["value"])}`;
			});
	}
	close() {
		this.readyState = 3;
		this.onclose?.();
	}
}

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

beforeEach(async () => {
	FakeCore.all = [];
	({ teardown } = installDom({ WebSocket: FakeCore, ResizeObserver: class { observe() {} unobserve() {} disconnect() {} } }));
	replaceDesks([{ id: "local", name: "Local", kind: "local" as const, origin: "ws://local", token: "local-key" }]);
	setActiveDesk("local");
	act = (await import("react")).act;
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
	await act(async () => {
		wireFor("local").connect();
	});
});

afterEach(async () => {
	await act(async () => {
		root.unmount();
		replaceDesks([]);
	});
	await teardown();
});

const SOL = "openai-codex/gpt-6.2-sol";
const effort: SessionConfig = { id: "effort", name: "Effort", category: "effort", currentId: "high", options: [{ id: "low", name: "Low" }, { id: "high", name: "High" }] };
const speeds = (ids: string[], currentId: string): SessionConfig => ({
	id: "fast",
	name: "Fast",
	category: "fast",
	currentId,
	options: [{ id: "off", name: "Standard" }, ...ids.map((id) => ({ id, name: id === "on" ? "Fast" : "Ultrafast" }))],
});

function entry({ backendId = "hotline", modelId = SOL, configs = [] as SessionConfig[], live = true, configChoices = {} as Record<string, string> }): RosterEntry {
	return {
		persona: {
			id: "sol", name: "Sol", goal: "", backendId, modelId, cwd: "/work", configChoices,
			mcpPolicy: { mode: "all", serverIds: [] }, skillPolicy: { mode: "all", names: [] },
			backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 0, updatedAt: 0,
		},
		session: {
			personaId: "sol", state: live ? "ready" : "idle", contextRestored: false, modes: [], configs, slashCommands: [],
			models: live ? [{ id: modelId, name: "GPT-6.2-Sol" }] : [],
			...(live ? { currentModelId: modelId } : {}),
			capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
		},
		waiting: false,
		drawing: false,
	};
}

async function show(row: RosterEntry, models: ConfigChoice[] = []) {
	await act(async () => {
		root.render(<SessionPickers entry={row} models={models} onSaid={() => {}} />);
	});
	await act(async () => container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!.click());
}
const menu = () => document.querySelector<HTMLElement>('[role="menu"][aria-label="Model"]')!;
const toggle = () => menu().querySelector<HTMLButtonElement>('[role="menuitemcheckbox"]');
const speedRow = () => menu().querySelector<HTMLFieldSetElement>('fieldset[aria-label="Speed"]');
const pill = () => container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!;
const core = () => FakeCore.all[0]!;

test("a ChatGPT model that lists Fast alone has the Fast switch, saying it uses more of the plan, and a flip asks for the tier", async () => {
	await show(entry({ configs: [effort, speeds(["on"], "off")] }));
	expect(speedRow()).toBeNull();
	expect(toggle()?.textContent).toBe("FastUses more of plan");
	expect(toggle()?.getAttribute("aria-checked")).toBe("false");
	await act(async () => toggle()!.click());
	expect(core().settings()).toEqual(["fast=on"]);
	expect(toggle()?.getAttribute("aria-checked")).toBe("true");
	expect(pill().querySelector(".model-pill-fast")).not.toBeNull();
});

test("a model that lists Ultrafast too has Standard, Fast and Ultrafast in the switch's place, and a choice asks for its tier", async () => {
	await show(entry({ configs: [effort, speeds(["on", "ultrafast"], "off")] }));
	expect(toggle()).toBeNull();
	const row = speedRow()!;
	expect(row.title).toBe("Uses more of plan");
	const choices = () => [...row.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')];
	expect(choices().map((one) => one.textContent)).toEqual(["Standard", "Fast", "Ultrafast"]);
	expect(choices().map((one) => one.getAttribute("aria-checked"))).toEqual(["true", "false", "false"]);
	expect(pill().querySelector(".model-pill-fast")).toBeNull();
	await act(async () => choices()[2]!.click());
	expect(core().settings()).toEqual(["fast=ultrafast"]);
	// Chosen at once, the pill wearing the bolt and naming the speed.
	expect(choices()[2]!.getAttribute("aria-checked")).toBe("true");
	expect(pill().querySelector(".model-pill-fast")).not.toBeNull();
	expect(pill().getAttribute("aria-label")).toContain(", ultrafast");
	await act(async () => choices()[0]!.click());
	expect(core().settings()).toEqual(["fast=ultrafast", "fast=off"]);
});

test("a model with no speed tier has no Fast row", async () => {
	await show(entry({ configs: [effort] }));
	expect(toggle()).toBeNull();
	expect(speedRow()).toBeNull();
});

const STANDARD = { id: "off", name: "Standard" };
const FAST = { id: "on", name: "Fast" };
const ULTRAFAST = { id: "ultrafast", name: "Ultrafast" };
const DAYBREAK = "openai-codex/gpt-daybreak-blue-latest";
const GPT6 = "openai-codex/gpt-6-sol";
// The models as the desk lists them at rest: each with the speeds its listing gives.
const resting: ConfigChoice[] = [
	{ id: SOL, name: "GPT-6.2-Sol", fast: true, speeds: [STANDARD, FAST, ULTRAFAST] },
	{ id: GPT6, name: "GPT-6-Sol", fast: true, speeds: [STANDARD, FAST] },
	{ id: DAYBREAK, name: "Daybreak Blue" },
];

async function again() {
	await act(async () => root.unmount());
	root = (await import("react-dom/client")).createRoot(container);
}

test("at rest a model the desk lists with Fast and Ultrafast has the speed choice, at the speed kept, and a choice is kept", async () => {
	await show(entry({ live: false, configChoices: { fast: "ultrafast" } }), resting);
	expect(toggle()).toBeNull();
	const row = speedRow()!;
	expect(row.title).toBe("Uses more of plan");
	const choices = () => [...row.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')];
	expect(choices().map((one) => one.textContent)).toEqual(["Standard", "Fast", "Ultrafast"]);
	expect(choices().map((one) => one.getAttribute("aria-checked"))).toEqual(["false", "false", "true"]);
	expect(pill().getAttribute("aria-label")).toContain(", ultrafast");
	await act(async () => choices()[1]!.click());
	expect(core().settings()).toEqual(["fast=on"]);
	await again();

	// Nothing kept, or a speed the model has not: Standard, as its session would start.
	for (const kept of [{}, { fast: "turbo" }]) {
		await show(entry({ live: false, configChoices: kept }), resting);
		const standard = speedRow()!.querySelector<HTMLButtonElement>('[role="menuitemradio"]')!;
		expect(standard.textContent).toBe("Standard");
		expect(standard.getAttribute("aria-checked")).toBe("true");
		expect(pill().querySelector(".model-pill-fast")).toBeNull();
		await again();
	}
});

test("at rest a model the desk lists with Fast alone has the switch, on for Fast kept, and none for a model with no speed", async () => {
	await show(entry({ live: false, modelId: GPT6, configChoices: { fast: "on" } }), resting);
	expect(speedRow()).toBeNull();
	expect(toggle()?.textContent).toBe("FastUses more of plan");
	expect(toggle()?.getAttribute("aria-checked")).toBe("true");
	await act(async () => toggle()!.click());
	expect(core().settings()).toEqual(["fast=off"]);
	await act(async () => toggle()!.click());
	expect(core().settings()).toEqual(["fast=off", "fast=on"]);
	await again();

	await show(entry({ live: false, modelId: DAYBREAK }), resting);
	expect(toggle()).toBeNull();
	expect(speedRow()).toBeNull();
});

test("the rule is every agent's: a harness Fast row with two speeds beyond off is the speed choice too", async () => {
	const row: SessionConfig = { id: "speed", name: "Speed", category: "fast", currentId: "off", options: [{ id: "off", name: "Off" }, { id: "on", name: "Fast" }, { id: "ultrafast", name: "Ultrafast" }] };
	await show(entry({ backendId: "codex-acp", modelId: "gpt-6.2-sol", configs: [row] }));
	expect(toggle()).toBeNull();
	await act(async () => speedRow()!.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')[2]!.click());
	expect(core().settings()).toEqual(["speed=ultrafast"]);
});

test("Codex's own Fast switch is as it was: on and off, its note its own", async () => {
	const codex = { id: "fast_mode", name: "Fast mode", category: "fast" as const, currentId: "on", options: [{ id: "on", name: "On" }, { id: "off", name: "Off" }] };
	await show(entry({ backendId: "codex-acp", modelId: "gpt-6.2-sol", configs: [codex] }));
	expect(toggle()?.textContent).toBe("FastUses more of plan");
	expect(toggle()?.getAttribute("aria-checked")).toBe("true");
	await act(async () => toggle()!.click());
	expect(core().settings()).toEqual(["fast_mode=off"]);
});
