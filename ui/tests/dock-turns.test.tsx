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

/** The handed-over thread as the desk holds it: Exec's, opened by Lead. */
let desk: { key: string; state: LinkState; working: boolean };
let threadHandlers: Handlers | undefined;
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;

function listed(): ThreadSummary[] {
	return [{
		thread: { kind: "side", key: desk.key },
		personaId: exec.id,
		title: "Add mul to calc.py",
		state: desk.state,
		...(desk.state === "closed" ? { end: "agent" as const, outcome: "Three tests pass." } : {}),
		opener: { personaId: lead.id, name: lead.name },
		startedAt: 1,
		updatedAt: 1,
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
	const connection = wireFor("local");
	const open = spyOn(connection, "onConnection").mockImplementation((watcher) => {
		watcher("open");
		return () => {};
	});
	restores.push(() => open.mockRestore());
	const subscription = spyOn(connection, "subscribe").mockImplementation(((target: { threadId?: { kind: string; key: string } }, handlers: Handlers) => {
		if (target.threadId?.key === desk.key) threadHandlers = handlers;
		const first: TranscriptEvent[] = target.threadId?.key === desk.key
			? [{ kind: "agent", id: "a1", ts: 2, text: "Three tests pass." }, { kind: "turn", id: "t1", ts: 3, stopReason: "end_turn" }]
			: [];
		queueMicrotask(() => handlers.snapshot(first));
		return () => {};
	}) as Wire["subscribe"]);
	restores.push(() => subscription.mockRestore());
	const command = spyOn(connection, "command").mockImplementation((async (cmd: string) => {
		if (cmd === "thread.list") return listed();
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
	return [entry(lead), { ...entry(exec), ...sides }];
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
