import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { LinkState, Persona, RosterEntry, SessionInfo, ThreadSummary, TranscriptEvent } from "../src/generated/contract";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Dock } = await import("../src/components/Dock");
const { replaceDesks, wireFor } = await import("../src/desks");

const persona = (id: string, name: string): Persona => ({
	id, name, goal: "", backendId: "hotline", cwd: `/work/${id}`,
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
});
const lead = persona("lead", "Lead");
const exec = persona("exec", "Exec");
/** Who the thread is between: a pane keeps a teammate's conversation open across tests, so a test that writes to it has teammates of its own. */
let sender = lead;
let receiver = exec;
const session = (personaId: string): SessionInfo => ({
	personaId, state: "idle", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
	capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
});

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Handlers = { snapshot(items: unknown[]): void; event(item: unknown): void };

/** The handed-over thread as the desk holds it: Exec's, opened by Lead, its handoff ended or standing, and when the list last heard from it. */
let desk: { key: string; state: LinkState; working: boolean; ended?: boolean; updatedAt?: number };
/** What the receiver's conversation already holds when the pane subscribes to it. */
let dmFirst: TranscriptEvent[];
let threadHandlers: Handlers | undefined;
let dmHandlers: Handlers | undefined;
/** The cancels the pane sent the desk, in order. */
let sent: { cmd: string; params: unknown }[];
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;

function listed(): ThreadSummary[] {
	return [{
		thread: { kind: "side", key: desk.key },
		personaId: receiver.id,
		title: "Add mul to calc.py",
		state: desk.state,
		...(desk.state === "closed" ? { end: "agent" as const, outcome: "Three tests pass." } : {}),
		opener: { personaId: sender.id, name: sender.name },
		...(desk.ended === true ? { handoffEnded: true } : {}),
		startedAt: 1,
		updatedAt: desk.updatedAt ?? 1,
		working: desk.working,
		waiting: false,
	}];
}

beforeEach(async () => {
	restores = [];
	const installed = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	// The pane hands focus back to the element it took it from.
	Object.assign(globalThis, { HTMLElement: installed.dom.HTMLElement });
	restores.push(() => Reflect.deleteProperty(globalThis, "HTMLElement"));
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	threadHandlers = undefined;
	dmHandlers = undefined;
	dmFirst = [];
	sender = lead;
	receiver = exec;
	const connection = wireFor("local");
	const open = spyOn(connection, "onConnection").mockImplementation((watcher) => {
		watcher("open");
		return () => {};
	});
	restores.push(() => open.mockRestore());
	const subscription = spyOn(connection, "subscribe").mockImplementation(((target: { threadId?: { kind: string; key: string } }, handlers: Handlers) => {
		if (target.threadId?.key === desk.key) threadHandlers = handlers;
		if (target.threadId?.kind === "dm") dmHandlers = handlers;
		const first: TranscriptEvent[] = target.threadId?.key === desk.key
			? [{ kind: "agent", id: "a1", ts: 2, text: "Three tests pass." }, { kind: "turn", id: "t1", ts: 3, stopReason: "end_turn" }]
			: target.threadId?.kind === "dm"
				? dmFirst
				: [];
		queueMicrotask(() => handlers.snapshot(first));
		return () => {};
	}) as Wire["subscribe"]);
	restores.push(() => subscription.mockRestore());
	sent = [];
	const command = spyOn(connection, "command").mockImplementation((async (cmd: string, params: unknown) => {
		if (cmd === "thread.list") return listed();
		if (cmd === "thread.cancel") {
			sent.push({ cmd, params });
			return null;
		}
		if (cmd === "thread.continue") {
			// The desk answers as soon as the thread is live again, while its agent is still starting.
			desk = { ...desk, state: "live", working: true };
			return listed()[0];
		}
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

/** The roster as the desk sends it: Exec's side threads, each working or not; none once it is closed. */
function roster(working: boolean | null): RosterEntry[] {
	const entry = (one: Persona): RosterEntry => ({ persona: one, waiting: false, drawing: false, session: session(one.id) });
	const sides = working === null ? {} : { sides: [{ sideId: desk.key, title: "Add mul to calc.py", startedAt: 1, working }] };
	return [entry(sender), { ...entry(receiver), ...sides }];
}

async function show(pane: Persona, working: boolean | null) {
	const all = roster(working);
	await act(async () => {
		root.render(
			<Dock
				state={{ open: { thread: { kind: "side", key: desk.key }, title: "Add mul to calc.py" } }}
				onState={() => {}}
				onClose={() => {}}
				onOpenTeammate={() => {}}
				entry={all.find((one) => one.persona.id === pane.id) ?? null}
				roster={all}
				width={360}
				onWidth={() => {}}
				overlay={false}
			/>,
		);
	});
	// The list and the thread's snapshot arrive on later ticks.
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

/** What says a turn is running: Stop in the pane's header, and Interrupt in its composer. */
const working = () => ({
	stop: container.querySelector('button[title="Stop this turn"]') !== null,
	interrupt: container.querySelector('[aria-label="Interrupt"]') !== null,
});
const idle = { stop: false, interrupt: false };

async function press(title: string) {
	const button = container.querySelector<HTMLButtonElement>(`button[title="${title}"]`);
	if (button === null) throw new Error(`No ${title} in: ${container.innerHTML}`);
	await act(async () => button.click());
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

describe("a handed-over thread continued in the pane", () => {
	for (const pane of [exec, lead]) {
		test(`is idle until a turn runs, working while it runs, and idle when it ends, in ${pane.name}'s pane`, async () => {
			desk = { key: `work-${pane.id}`, state: "closed", working: false };
			await show(pane, null);
			expect(container.querySelector('button[title="Resume this thread"]') !== null).toBe(true);

			await press("Resume this thread");
			// The roster hears the agent start, then come up with nothing to say.
			await show(pane, true);
			desk = { ...desk, working: false };
			await show(pane, false);
			expect(container.querySelector('button[title="Resume this thread"]') !== null).toBe(false);
			expect(container.querySelector("textarea") !== null).toBe(true);
			expect(working()).toEqual(idle);

			// The person says something: a turn runs.
			desk = { ...desk, working: true };
			await act(async () => threadHandlers?.event({ kind: "user", id: "u1", ts: 4, text: "Add div too." }));
			await show(pane, true);
			expect(working()).toEqual({ stop: true, interrupt: true });
			expect(container.textContent).toContain("Thinking");

			// The turn ends.
			desk = { ...desk, working: false };
			await act(async () => {
				threadHandlers?.event({ kind: "agent", id: "a2", ts: 5, text: "Added div." });
				threadHandlers?.event({ kind: "turn", id: "t2", ts: 6, stopReason: "end_turn" });
			});
			await show(pane, false);
			expect(working()).toEqual(idle);
		});
	}
});

describe("a handed-over thread's turn", () => {
	test("is stopped, with the work it does, by Stop in the header, and only cut by the composer's Interrupt or Esc", async () => {
		desk = { key: "work-cut", state: "live", working: true };
		await show(exec, true);
		const thread = { kind: "side", key: desk.key };
		await press("Stop this turn");
		expect(sent).toEqual([{ cmd: "thread.cancel", params: { thread } }]);

		sent = [];
		const interrupt = container.querySelector<HTMLButtonElement>('[aria-label="Interrupt"]');
		if (interrupt === null) throw new Error(`No Interrupt in: ${container.innerHTML}`);
		await act(async () => interrupt.click());
		const field = container.querySelector("textarea");
		if (field === null) throw new Error(`No composer in: ${container.innerHTML}`);
		await act(async () => {
			field.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
		});
		expect(sent).toEqual([
			{ cmd: "thread.cancel", params: { thread, interrupt: true } },
			{ cmd: "thread.cancel", params: { thread, interrupt: true } },
		]);
	});
});

/** The thread's link on the receiver's tape, as the desk rewrites it. */
const linkOf = (ended: boolean): TranscriptEvent => ({
	kind: "link",
	id: `link:side:${desk.key}`,
	ts: 1,
	thread: desk.key,
	threadKind: "side",
	personaId: receiver.id,
	title: "Add mul to calc.py",
	state: desk.state,
	openerId: sender.id,
	openerName: sender.name,
	...(ended ? { handoffEnded: true } : {}),
});

describe("the bar of a handed-over thread", () => {
	test("says what the receiver says reaches the sender while the handoff stands, and that it no longer does once the desk ended it", async () => {
		sender = persona("poe", "Poe");
		receiver = persona("toad", "Toad");
		// The thread's link was on Toad's tape before the pane opened, and the list has heard from the thread since,
		// so the rewritten link alone is older than the row: only the list, read again for the link, can say it ended.
		desk = { key: "work-bar", state: "live", working: false, updatedAt: 5 };
		dmFirst = [linkOf(false)];
		await show(receiver, false);
		expect(dmHandlers).toBeDefined();
		expect(container.textContent).toContain("Poe handed this to Toad. What Toad says here reaches Poe.");

		// The desk records the end on the thread's link, and the list read again says so.
		desk = { ...desk, ended: true };
		await act(async () => dmHandlers?.event(linkOf(true)));
		await show(receiver, false);
		expect(container.textContent).toContain("Poe handed this to Toad. The handoff has ended: what Toad says here no longer reaches Poe.");
		expect(container.textContent).not.toContain("What Toad says here reaches Poe.");
		expect(container.textContent).toContain("Talk to Poe about it");
	});
});
