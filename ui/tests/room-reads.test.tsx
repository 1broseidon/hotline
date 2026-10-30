import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { RosterEntry } from "../src/wire";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }) } });
const { SessionPickers } = await import("../src/components/Pickers");
const { Teammate } = await import("../src/components/Teammate");
const { useRoomSettings } = await import("../src/room");
const { replaceDesks, setActiveDesk, wireFor } = await import("../src/desks");

const entry = (id: string, backendId = "hotline", modelId?: string): RosterEntry => ({
	persona: {
		id, name: id, goal: "", backendId, ...(modelId === undefined ? {} : { modelId }), cwd: "/work",
		mcpPolicy: { mode: "all", serverIds: [] }, skillPolicy: { mode: "all", names: [] },
		backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 0, updatedAt: 0,
	},
	session: {
		personaId: id, state: "idle", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
		capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
	}, waiting: false, drawing: false,
});
const noop = () => {};

class FakeCore {
	static readonly OPEN = 1;
	static all: FakeCore[] = [];
	readyState = 1;
	sent: Record<string, unknown>[] = [];
	onopen: (() => void) | null = null;
	onmessage: ((message: { data: string }) => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	label: string;
	hold = false;
	waiting: Record<string, unknown>[] = [];
	constructor(readonly url: string) {
		this.label = url.includes("remote") ? "Remote" : "Local";
		FakeCore.all.push(this);
		queueMicrotask(() => this.onopen?.());
	}
	reply(frame: Record<string, unknown>) { this.onmessage?.({ data: JSON.stringify(frame) }); }
	send(data: string) {
		const frame = JSON.parse(data) as Record<string, unknown>;
		this.sent.push(frame);
		queueMicrotask(() => {
			if ("sub" in frame) this.reply({ sub: frame["id"], snapshot: [] });
			if (!("cmd" in frame)) return;
			const label = this.label;
			const answers: Record<string, unknown> = {
				"client.hello": { capabilities: ["threads2"] },
				"models.efforts": { choices: [{ id: "high", name: `${label} effort` }], defaultId: "high" },
				"backends.list": [{ id: "harness", name: `${label} harness` }],
				"voice.status": { front: { providerId: label, modelId: "voice", voices: [label.toLowerCase()] } },
				"capabilities.options": { tts: { options: [] } },
				"computer.status": { state: "absent" },
				"skills.list": [], "teammate.tools": null,
				// A model set at rest answers with the row, its effort picker on the new model.
				"session.set_model": { configs: [{ id: "reasoning_effort", name: "Reasoning effort", category: "effort", currentId: "medium", options: [] }] },
			};
			const reply = { id: frame["id"], ok: true, result: answers[String(frame["cmd"])] ?? null };
			if (this.hold && frame["cmd"] === "models.efforts") this.waiting.push(reply);
			else this.reply(reply);
		});
	}
	count(cmd: string) { return this.sent.filter((frame) => frame["cmd"] === cmd).length; }
	room(event: Record<string, unknown>) {
		const sub = this.sent.findLast((frame) => frame["sub"] === "room")!;
		this.reply({ sub: sub["id"], event });
	}
	close() { this.readyState = 3; this.onclose?.(); }
}

let act: typeof import("react").act;
let useLayoutEffect: typeof import("react").useLayoutEffect;
let layouts: string[];
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
const local = { id: "local", name: "Local", kind: "local" as const, origin: "ws://local", token: "local-key" };
const remote = { id: "remote", name: "Remote", kind: "remote" as const, origin: "ws://remote", token: "remote-key" };

beforeEach(async () => {
	FakeCore.all = [];
	layouts = [];
	({ teardown } = installDom({ WebSocket: FakeCore, ResizeObserver: class { observe() {} unobserve() {} disconnect() {} } }));
	replaceDesks([local, remote]);
	setActiveDesk("local");
	({ act, useLayoutEffect } = await import("react"));
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
	await act(async () => { wireFor("local").connect(); wireFor("remote").connect(); });
});
afterEach(async () => {
	await act(async () => { root.unmount(); replaceDesks([]); });
	await teardown();
});

function Pane({ row }: { row: RosterEntry }) {
	// The App keeps the room watched while the keyed teammate changes.
	useRoomSettings();
	useLayoutEffect(() => { layouts.push(container.textContent ?? ""); });
	return <>
		<SessionPickers key={`picker-${row.persona.id}`} entry={row} models={[{ id: "model", name: "Model" }, { id: "other", name: "Other" }]} onSaid={noop} />
		<Teammate key={row.persona.id} persona={row.persona} session={row.session} jobs={[]} roster={[row]}
			focusSchedules={false} onClose={noop} onDeleted={noop} onOpenThread={noop} />
	</>;
}
async function show(row: RosterEntry, desk = "local") {
	await act(async () => { setActiveDesk(desk); root.render(<Pane key={desk} row={row} />); });
}
const core = () => FakeCore.all[0]!;
const text = () => container.textContent ?? "";

test("repeated teammate switches share metadata while each computer status stays fresh", async () => {
	for (let i = 0; i < 10; i++) {
		await show(entry(`agent-${i}`));
		if (i > 0) expect(layouts.at(-1)).toContain("Local effort");
		expect(text()).toContain("Local effort");
	}
	for (let i = 0; i < 10; i++) {
		await show(entry(`harness-${i}`, "harness"));
		if (i > 0) expect(layouts.at(-1)).toContain("Local harness");
		expect(text()).toContain("Local harness");
	}
	const counts = Object.fromEntries(["models.efforts", "backends.list", "voice.status", "capabilities.options", "computer.status"].map((cmd) => [cmd, core().count(cmd)]));
	console.log("20 teammate selections:", JSON.stringify(counts));
	expect(counts).toEqual({ "models.efforts": 1, "backends.list": 1, "voice.status": 1, "capabilities.options": 1, "computer.status": 20 });
});

test("provider/model and settings changes refresh mounted choices; unrelated room events do not", async () => {
	await show(entry("a"));
	await act(async () => { core().room({ kind: "setting", id: "lastModelId", value: "model" }); });
	await act(async () => { core().room({ kind: "schedule", id: "job", deleted: true }); });
	expect(core().count("models.efforts")).toBe(1);
	for (const event of [{ kind: "credential", id: "provider", deleted: true }, { kind: "models", id: "provider" }, { kind: "setting", id: "enabledModels", value: {} }, { kind: "setting", id: "voice", value: {} }]) {
		core().label += "X";
		await act(async () => { core().room(event); });
		expect(text()).toContain(`${core().label} effort`);
		expect(text()).toContain(core().label.toLowerCase().replace(/^l/, "L"));
	}
	expect(core().count("models.efforts")).toBe(5);
	await show(entry("b", "hotline", "other"));
	expect(core().count("models.efforts")).toBe(6);
	await show(entry("a"));
	expect(core().count("models.efforts")).toBe(6);
});

test("remote desks and replaced endpoints cannot reuse local or previous credentials' data", async () => {
	await show(entry("a"));
	await show(entry("a"), "remote");
	expect(text()).toContain("Remote effort");
	expect(text()).not.toContain("Local effort");
	await show(entry("a"));
	expect(text()).toContain("Local effort");
	await act(async () => { replaceDesks([{ ...local, origin: "ws://replacement", token: "new-key" }, remote]); wireFor("local").connect(); });
	FakeCore.all.at(-1)!.label = "Replacement";
	await show(entry("b"));
	expect(text()).toContain("Replacement effort");
	expect(text()).not.toContain("Local effort");
});

test("late replies cannot restore an invalidated model list", async () => {
	core().hold = true;
	await show(entry("a"));
	await show(entry("b"));
	expect(core().count("models.efforts")).toBe(1);
	core().hold = false;
	core().label = "New";
	await act(async () => { core().room({ kind: "models", id: "provider" }); });
	expect(text()).toContain("New effort");
	await act(async () => { for (const reply of core().waiting) core().reply(reply); });
	expect(text()).toContain("New effort");
	expect(text()).not.toContain("Local effort");
});


test("a reconnect snapshot discards cached reads and a refused read retries on remount", async () => {
	core().hold = true;
	await show(entry("a"));
	await act(async () => { core().reply({ ...core().waiting[0], ok: false, error: "Unavailable" }); });
	expect(text()).not.toContain("Local effort");
	core().hold = false;
	await show(entry("b"));
	expect(text()).toContain("Local effort");
	await act(async () => { core().close(); });
	expect(text()).not.toContain("Local effort");
	await act(async () => { wireFor("local").connect(); });
	const reconnected = FakeCore.all.at(-1)!;
	expect(reconnected.count("models.efforts")).toBe(1);
	expect(text()).toContain("Local effort");
});

test("a Hotline Agent teammate with no account left keeps the picker, so it can still move to an installed agent", async () => {
	await act(async () => {
		setActiveDesk("local");
		root.render(<SessionPickers key="no-accounts" entry={entry("no-accounts")} models={[]} onSaid={noop} />);
	});
	await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
	const button = container.querySelector<HTMLButtonElement>("button.model-pill");
	expect(button).not.toBeNull();
	await act(async () => button!.click());
	await act(async () => document.querySelector<HTMLButtonElement>(".model-change")!.click());
	expect(document.querySelector('[role="menu"][aria-label="Change agent"]')?.textContent).toContain("Agents");
});

test("a resting harness teammate's strip keeps the pretty name and effort its last live session named, never the model id", async () => {
	const live = entry("resting", "harness", "claude-sonnet-5-5");
	live.session = {
		...live.session,
		state: "ready",
		currentModelId: "claude-sonnet-5-5",
		models: [{ id: "claude-sonnet-5-5", name: "Sonnet 5.5" }],
		configs: [{ id: "effort", name: "Effort", category: "effort", currentId: "high", options: [{ id: "medium", name: "Medium" }, { id: "high", name: "High" }] }],
	};
	await act(async () => {
		setActiveDesk("local");
		root.render(<SessionPickers key="live" entry={live} models={[]} onSaid={noop} />);
	});
	await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
	expect(container.querySelector("button.model-pill")?.textContent).toContain("Sonnet 5.5 · High");

	await act(async () => { root.render(<SessionPickers key="resting" entry={entry("resting", "harness", "claude-sonnet-5-5")} models={[]} onSaid={noop} />); });
	await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
	const resting = container.textContent ?? "";
	expect(resting).toContain("Sonnet 5.5 · High");
	expect(resting).not.toContain("claude-sonnet-5-5");
});

test("opening the model picker asks the core to read the teammate's plan again", async () => {
	const live = entry("planned", "harness", "claude-sonnet-5-5");
	live.session = { ...live.session, state: "ready", currentModelId: "claude-sonnet-5-5", models: [{ id: "claude-sonnet-5-5", name: "Sonnet 5.5" }] };
	await act(async () => {
		setActiveDesk("local");
		root.render(<SessionPickers key="planned" entry={live} models={[]} onSaid={noop} />);
	});
	await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
	expect(core().count("plan.refresh")).toBe(0);
	await act(async () => container.querySelector<HTMLButtonElement>("button.model-pill")!.click());
	const asked = core().sent.filter((frame) => frame["cmd"] === "plan.refresh");
	expect(asked.map((frame) => frame["params"])).toEqual([{ personaId: "planned" }]);
});

test("a resting harness teammate's menu lists the models and effort its agent last offered, and effort chosen at rest is set and shown", async () => {
	// The roster row as the core draws a teammate that is not running: its
	// agent's last offer, the teammate's own choices current.
	const resting = entry("rested", "harness", "sonnet");
	const offered = (effort: string): RosterEntry => ({
		...resting,
		session: {
			...resting.session,
			models: [{ id: "opus", name: "Opus 5.5" }, { id: "sonnet", name: "Sonnet 5.5" }],
			currentModelId: "sonnet",
			configs: [{ id: "reasoning", name: "Effort", category: "effort", currentId: effort, options: [{ id: "low", name: "Low" }, { id: "medium", name: "Medium" }, { id: "high", name: "High" }] }],
		},
	});
	await act(async () => {
		setActiveDesk("local");
		root.render(<SessionPickers key="rested" entry={offered("high")} models={[]} onSaid={noop} />);
	});
	await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
	const pill = () => container.querySelector<HTMLButtonElement>("button.model-pill")!;
	expect(pill().textContent).toContain("Sonnet 5.5 · High");

	await act(async () => pill().click());
	const main = document.querySelector<HTMLElement>(".model-menu-main")!;
	const rows = [...main.querySelectorAll<HTMLElement>('.model-row[role="menuitemradio"]')];
	expect(rows.map((row) => [row.textContent, row.getAttribute("aria-checked")])).toEqual([
		["Opus 5.5", "false"],
		["Sonnet 5.5", "true"],
	]);
	const tabs = () => [...main.querySelectorAll<HTMLElement>('[aria-label="Effort"] [role="menuitemradio"]')];
	expect(tabs().map((one) => one.textContent)).toEqual(["Low", "Med", "High"]);
	await act(async () => tabs()[0]!.click());
	const asked = core().sent.filter((frame) => frame["cmd"] === "session.set_config");
	expect(asked.map((frame) => frame["params"])).toEqual([{ personaId: "rested", configId: "reasoning", value: "low" }]);
	expect(document.querySelector(".model-menu-main")).not.toBeNull();

	// The core answers by drawing the row again with the choice current.
	await act(async () => { root.render(<SessionPickers key="rested" entry={offered("low")} models={[]} onSaid={noop} />); });
	expect(pill().textContent).toContain("Sonnet 5.5 · Low");
});

test("a resting teammate on an agent never run here shows the models and levels the desk read out of band, and a model switch shows the new model's tabs at the effort the core landed on", async () => {
	// The row as the core draws it from Codex's catalogue alone: each model
	// with its levels, the effort picker on the teammate's model.
	const resting = entry("fresh", "codex-acp");
	const levels = (ids: string[]) => ids.map((id) => ({ id, name: id[0]!.toUpperCase() + id.slice(1) }));
	resting.session = {
		...resting.session,
		models: [
			{ id: "gpt-6.1-sol", name: "6.1 Sol", efforts: levels(["low", "medium", "xhigh"]), defaultEffort: "low" },
			{ id: "gpt-6-luna", name: "6 Luna", efforts: levels(["medium", "max"]), defaultEffort: "medium" },
		],
		currentModelId: "gpt-6.1-sol",
		configs: [{ id: "reasoning_effort", name: "Reasoning effort", category: "effort", currentId: "low", options: levels(["low", "medium", "xhigh"]) }],
	};
	await act(async () => {
		setActiveDesk("local");
		root.render(<SessionPickers key="fresh" entry={resting} models={[]} onSaid={noop} />);
	});
	await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
	const pill = container.querySelector<HTMLButtonElement>("button.model-pill")!;
	expect(pill.textContent).toContain("6.1 Sol · Low");

	await act(async () => pill.click());
	const main = document.querySelector<HTMLElement>(".model-menu-main")!;
	const tabs = () => [...main.querySelectorAll<HTMLElement>('[aria-label="Effort"] [role="menuitemradio"]')];
	expect(tabs().map((one) => one.textContent)).toEqual(["Low", "Med", "Xhigh"]);
	const rows = [...main.querySelectorAll<HTMLElement>('.model-row[role="menuitemradio"]')];
	expect(rows.map((row) => row.getAttribute("aria-haspopup"))).toEqual([null, null]);
	await act(async () => rows[1]!.click());
	await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
	// The core chooses the effort: one command, and the menu shows Luna's tabs at its answer.
	const asked = core().sent.filter((frame) => frame["cmd"] === "session.set_model" || frame["cmd"] === "session.set_config");
	expect(asked.map((frame) => [frame["cmd"], frame["params"]])).toEqual([["session.set_model", { personaId: "fresh", modelId: "gpt-6-luna" }]]);
	expect(document.querySelector(".model-menu-main")).not.toBeNull();
	expect(tabs().map((one) => [one.textContent, one.getAttribute("aria-checked")])).toEqual([
		["Med", "true"],
		["Max", "false"],
	]);
});
