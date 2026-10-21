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
