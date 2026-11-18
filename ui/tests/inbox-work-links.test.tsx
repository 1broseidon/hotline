import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { InboxItem, InboxView, Persona, RosterEntry, SessionInfo, ThreadSummary, TranscriptEvent } from "../src/generated/contract";
import type { DockState } from "../src/components/Dock";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

// task-501: an inbox item taken into work and its work thread open each
// other, from the item's row in the inbox pane, from the item's pane, and
// from the work thread's pane, and Back goes to where the person came from.

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Dock } = await import("../src/components/Dock");
const { replaceDesks, wireFor } = await import("../src/desks");

const NOW = Date.now();
const persona = (id: string, name: string): Persona => ({
	id, name, goal: "", backendId: "hotline", cwd: `/work/${id}`,
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
});
const goldie = persona("goldie", "Goldie");
const huck = persona("huck", "Huck");
const session = (personaId: string): SessionInfo => ({
	personaId, state: "idle", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
	capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
});
const WORK = { kind: "side" as const, key: "s-linux" };
const TITLE = "Rerun the Linux build";

/** Goldie's item in Huck's inbox, which Huck took into a work thread. */
const taken: InboxItem = {
	id: "i-linux", ts: NOW - 600_000, from: goldie.id, fromThread: { kind: "dm", key: goldie.id }, to: huck.id,
	title: TITLE, body: "The Linux build went red on the packaging step. Rerun it and tell me if it is the runner.",
	wants: "work", state: "taken", work: WORK, request: "r-linux", senderTold: 3,
	receipts: [
		{ state: "new", at: NOW - 600_000 },
		{ state: "seen", at: NOW - 590_000 },
		{ state: "taken", at: NOW - 580_000 },
	],
};
const view: InboxView = { personaId: huck.id, open: [taken], closed: [], sent: [], unread: 0 };
/** The work thread's row, as `thread.list` sends it: opened with the item's sender as its opener. */
const row: ThreadSummary = {
	thread: WORK, personaId: huck.id, title: TITLE, state: "live", opener: { personaId: goldie.id, name: goldie.name },
	startedAt: NOW - 580_000, updatedAt: NOW - 60_000, working: true, waiting: false,
};
/** The thread's first line is the item, as the desk seeds it. */
const seeded: TranscriptEvent = {
	kind: "delivery", id: "handoff:r-linux", ts: NOW - 580_000, text: `${TITLE}\nFrom Goldie, item i-linux, wants work:\n${taken.body}`,
	cause: { kind: "handoff", requestId: "r-linux", personaId: goldie.id, name: goldie.name, threadKey: "goldie~huck", about: TITLE },
	from: { kind: "dm", thread: goldie.id, request: "r-linux" },
};

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
let states: DockState[];

beforeEach(async () => {
	restores = [];
	states = [];
	const installed = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	Object.assign(globalThis, { HTMLElement: installed.dom.HTMLElement });
	restores.push(() => Reflect.deleteProperty(globalThis, "HTMLElement"));
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	const connection = wireFor("local");
	const open = spyOn(connection, "onConnection").mockImplementation((watcher) => {
		watcher("open");
		return () => {};
	});
	restores.push(() => open.mockRestore());
	type Handlers = { snapshot(items: unknown): void };
	const subscription = spyOn(connection, "subscribe").mockImplementation(((target: { inbox?: string; threadId?: { kind: string; key: string } }, handlers: Handlers) => {
		const first = target.inbox === huck.id ? view : target.inbox !== undefined ? { ...view, personaId: target.inbox, open: [] } : target.threadId?.key === WORK.key ? [seeded] : [];
		queueMicrotask(() => handlers.snapshot(first));
		return () => {};
	}) as Wire["subscribe"]);
	restores.push(() => subscription.mockRestore());
	const command = spyOn(connection, "command").mockImplementation((async (cmd: string) => {
		if (cmd === "thread.list") return [row];
		if (cmd === "aside.list") return [];
		if (cmd === "inbox.item") return { item: taken, replies: [] };
		throw new Error(`Not in this test: ${cmd}`);
	}) as Wire["command"]);
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

function roster(): RosterEntry[] {
	const entry = (one: Persona): RosterEntry => ({ persona: one, waiting: false, drawing: false, session: session(one.id) });
	return [entry(goldie), { ...entry(huck), sides: [{ sideId: WORK.key, title: TITLE, startedAt: NOW - 580_000, working: true }] }];
}

async function show(state: DockState) {
	const all = roster();
	await act(async () => {
		root.render(
			<Dock
				state={state}
				onState={(next) => states.push(next)}
				onClose={() => {}}
				onOpenTeammate={() => {}}
				onOpenTeammateThread={() => {}}
				entry={all.find((one) => one.persona.id === huck.id) ?? null}
				roster={all}
				width={360}
				onWidth={() => {}}
				overlay={false}
			/>,
		);
	});
	for (let tick = 0; tick < 3; tick++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
	}
}

async function press(element: Element | null) {
	if (element === null) throw new Error(`Not drawn: ${container.innerHTML}`);
	await act(async () => (element as HTMLElement).click());
}

describe("task-501 a taken message and its work thread", () => {
	test("the message's row in the inbox says its work is running and opens its topic, where the work thread is a row under the message", async () => {
		const inbox: DockState = { open: null, inbox: {} };
		await show(inbox);
		const row = container.querySelector("[data-topic]");
		expect(row?.querySelector(".dock-dot")).not.toBeNull();
		await press(row);
		expect(states.at(-1)).toEqual({ open: null, inbox: { item: "i-linux" }, from: inbox });
	});

	test("the message's pane opens its work thread, and Back comes to the message", async () => {
		const item: DockState = { open: null, inbox: { item: "i-linux" }, from: { open: null, inbox: {} } };
		await show(item);
		const work = container.querySelector("[data-work=i-linux]");
		expect(work?.textContent).toBe("≡Work thread · Running");
		await press(work);
		expect(states.at(-1)).toEqual({ open: { thread: WORK, title: TITLE }, from: item });
	});

	test("the work thread's pane opens its message, and Back from each goes where the person came from", async () => {
		const inbox: DockState = { open: null, inbox: {} };
		const thread: DockState = { open: { thread: WORK, title: TITLE }, from: inbox };
		await show(thread);
		// A work thread, with the message over it, not a handoff.
		expect(container.querySelector(".band-kind")?.textContent).toBe("Work thread");
		expect(container.querySelector(".pane-task .spine-rule-title")?.textContent).toBe("Message");
		// The item's title keeps its line over its words; the line the desk hands the teammate is not the person's to read.
		expect(container.querySelectorAll(".pane-task br")).toHaveLength(1);
		const message = container.querySelector(".pane-task .spine-clamp")?.textContent ?? "";
		expect(message).toStartWith(TITLE);
		expect(message).toContain(taken.body);
		expect(message).not.toContain("From Goldie");
		expect(message).not.toContain("i-linux");
		// The thread is with its teammate.
		expect([...container.querySelectorAll(".spine-facts dd")].map((one) => one.textContent)).toEqual(["Huck", expect.any(String)]);
		await press(container.querySelector('.pane-task button[title="Open the message"]'));
		const item = states.at(-1)!;
		expect(item).toEqual({ open: null, inbox: { item: "i-linux" }, from: thread });
		await show(item);
		expect(container.querySelector(".spine-title")?.textContent).toBe(TITLE);
		await press(container.querySelector('.band button[title="Back"]'));
		expect(states.at(-1)).toEqual(thread);
		await show(thread);
		await press(container.querySelector('.band button[title="Back"]'));
		expect(states.at(-1)).toEqual(inbox);
	});
});
