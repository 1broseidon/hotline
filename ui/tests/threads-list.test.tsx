import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Aside, Persona, RosterEntry, RunningSubagent, ScheduleEntry, SessionInfo, ThreadSummary, TranscriptEvent } from "../src/generated/contract";
import type { DockState } from "../src/components/Dock";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

// epic-38: one Threads list (docs/design/threads-list.html, idea A).

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Dock } = await import("../src/components/Dock");
const { replaceDesks, wireFor } = await import("../src/desks");
const { rememberFilter } = await import("../src/threadList");

const persona = (id: string, name: string): Persona => ({
	id, name, goal: "", backendId: "hotline", cwd: `/work/${id}`,
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
});
const session = (personaId: string): SessionInfo => ({
	personaId, state: "idle", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
	capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
});
const pork = persona("pork", "Pork Chop");
const elvis = persona("elvis", "Elvis");
const lou = persona("lou", "Lou");
const entry = (one: Persona, extra: Partial<RosterEntry> = {}): RosterEntry => ({ persona: one, waiting: false, drawing: false, session: session(one.id), ...extra });

/** Today at a clock time, so a row's time reads as that clock. */
const today = (hours: number, minutes = 0) => {
	const at = new Date();
	at.setHours(hours, minutes, 0, 0);
	return Math.min(at.getTime(), Date.now() - 60_000);
};

const summary = (row: Partial<ThreadSummary> & Pick<ThreadSummary, "thread">): ThreadSummary => ({
	personaId: "pork", state: "live", startedAt: today(0), updatedAt: today(0, 30), working: false, waiting: false, ...row,
});
const aside = (id: string, ts: number, extra: Partial<Aside> = {}): Aside => ({
	id, ts, to: "pork", from: { thread: { kind: "job", key: "watch" } }, by: "pork", text: `aside ${id}`, level: "note", ...extra,
});
const job = (id: string, extra: Partial<ScheduleEntry> = {}): ScheduleEntry => ({ id, personaId: "pork", name: id, kind: "loop", prompt: id, nextAt: Date.now() + 240_000, ...extra });

const audit: RunningSubagent = { runId: "r-audit", title: "Audit side-pane thread rows", startedAt: 1, lastStep: "Read Dock.tsx", steps: 1 };
const precheck: RunningSubagent = { runId: "r-pre", title: "make precheck", startedAt: 1, kind: "shell", command: "make precheck" };

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Handlers = { snapshot(items: unknown[]): void; event(item: unknown): void };
/** What the desk answers: the list, the jobs, the notes; and what the window sent it. */
let listed: ThreadSummary[];
let jobs: ScheduleEntry[];
let asides: Aside[];
let threadLines: Record<string, TranscriptEvent[]>;
let sent: { cmd: string; params: unknown }[];
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;

beforeEach(async () => {
	restores = [];
	const installed = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	Object.assign(globalThis, { HTMLElement: installed.dom.HTMLElement });
	restores.push(() => Reflect.deleteProperty(globalThis, "HTMLElement"));
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	listed = [];
	jobs = [];
	asides = [];
	threadLines = {};
	sent = [];
	for (const one of ["pork", "elvis", "lou"]) rememberFilter(one, "all");
	const connection = wireFor("local");
	const open = spyOn(connection, "onConnection").mockImplementation((watcher) => {
		watcher("open");
		return () => {};
	});
	restores.push(() => open.mockRestore());
	const subscription = spyOn(connection, "subscribe").mockImplementation(((target: unknown, handlers: Handlers) => {
		const first =
			typeof target === "object" && target !== null && "schedules" in target
				? jobs
				: typeof target === "object" && target !== null && "threadId" in target
					? (threadLines[(target as { threadId: { key: string } }).threadId.key] ?? [])
					: [];
		queueMicrotask(() => handlers.snapshot(first));
		return () => {};
	}) as Wire["subscribe"]);
	restores.push(() => subscription.mockRestore());
	const command = spyOn(connection, "command").mockImplementation((async (cmd: string, params: unknown) => {
		sent.push({ cmd, params });
		if (cmd === "thread.list") return listed;
		if (cmd === "aside.list") return asides;
		if (cmd === "aside.seen" || cmd === "thread.answer" || cmd === "secrets.passkey.answer") return null;
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

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

/** The pane, on `roster[0]`'s list; what it asked to show next is in `states`. */
async function dock(roster: RosterEntry[], states: DockState[] = []) {
	await act(async () => {
		root.render(
			<Dock
				state={{ open: null }}
				onState={(next) => states.push(next)}
				onClose={() => {}}
				onOpenTeammate={() => {}}
				entry={roster[0] ?? null}
				roster={roster}
				width={360}
				onWidth={() => {}}
				overlay={false}
			/>,
		);
	});
	await settle();
	await settle();
}

const row = (id: string) => container.querySelector<HTMLElement>(`.list-row[data-thread="${id}"]`);
const rowIds = () => [...container.querySelectorAll(".list-row")].map((one) => one.getAttribute("data-thread"));
const parts = (id: string) => {
	const one = row(id);
	return {
		mark: one?.querySelector(".list-mark")?.textContent,
		kind: one?.querySelector(".list-mark")?.getAttribute("title"),
		title: one?.querySelector(".list-title-text")?.textContent,
		line: one?.querySelector(".list-line")?.textContent,
		when: one?.querySelector(".list-when")?.textContent,
		beats: one?.querySelector(".list-when .dock-dot.beat") !== null && one?.querySelector(".list-when .dock-dot.beat") !== undefined,
	};
};
const chip = (word: string) => [...container.querySelectorAll<HTMLButtonElement>(".list-chips button")].find((one) => one.textContent?.startsWith(word));

/** One of every kind, as the mockup has them. */
function everyKind() {
	listed = [
		summary({ thread: { kind: "side", key: "s1" }, title: "Release notes for 0.49", working: true, preview: "Yes, and keep it short." }),
		summary({ thread: { kind: "side", key: "h1" }, personaId: "pork", title: "Check the 0.49 site copy", opener: { personaId: "elvis", name: "Elvis" }, preview: "Three lines to approve." }),
		summary({ thread: { kind: "run", key: "r-audit" }, title: "Audit side-pane thread rows", working: true }),
		summary({ thread: { kind: "run", key: "r-pre" }, title: "make precheck", working: true }),
		summary({ thread: { kind: "pair", key: "lou~pork" }, title: "Pork Chop and Lou", state: "closed", end: "done", outcome: "Agreed the button order for task-71" }),
		summary({ thread: { kind: "call", key: "c1" }, title: "Device-code login test", state: "closed", end: "person", outcome: "2 min" }),
		summary({ thread: { kind: "job", key: "watch" }, title: "Watch CI", working: true }),
		summary({ thread: { kind: "dm", key: "pork" } }),
	];
	jobs = [job("watch", { name: "Watch CI on release/0.49.0" })];
	asides = [aside("n1", today(0, 40), { text: "Linux and Windows are green on 2736d08d." })];
	const sides = [{ sideId: "s1", title: "Release notes for 0.49", startedAt: 1, working: true }, { sideId: "h1", title: "Check the 0.49 site copy", startedAt: 1, working: false }];
	return [entry(pork, { subagents: [audit, precheck], workingJobs: ["watch"], sides }), entry(elvis), entry(lou)];
}

describe("task-400 one row shape", () => {
	test("The pane titled Threads lists work threads, handoffs, subagents, commands, pairs, calls, jobs and notes (asides) as rows of one shape: kind mark, title, one line (state, latest step, or result), time; a live row shows its latest step and a beating dot", async () => {
		await dock(everyKind());
		expect(container.querySelector(".dock")?.getAttribute("aria-label")).toBe("Threads");
		expect(container.querySelector(".band h2")?.textContent).toBe("Threads");
		// The teammate's own conversation is the window beside the list, not a row.
		expect(row("dm:pork")).toBeNull();
		const marks = Object.fromEntries(
			["side:s1", "side:h1", "run:r-audit", "run:r-pre", "pair:lou~pork", "call:c1", "job:watch", "note:n1"].map((id) => [id, `${parts(id).mark} ${parts(id).kind}`]),
		);
		expect(marks).toEqual({
			"side:s1": "≡ Work thread",
			"side:h1": "⇄ Handoff",
			"run:r-audit": "◆ Subagent",
			"run:r-pre": "$ Command",
			"pair:lou~pork": "◎ Pair",
			"call:c1": "☏ Call",
			"job:watch": "↻ Job",
			"note:n1": "✎ Note",
		});
		// Every row is the same four parts.
		for (const one of container.querySelectorAll(".list-row")) {
			expect(one.querySelector(".list-mark")).not.toBeNull();
			expect(one.querySelector(".list-title-text")?.textContent).not.toBe("");
			expect(one.querySelector(".list-line")?.textContent).not.toBe("");
			expect(one.querySelector(".list-when")?.textContent).toMatch(/\d/);
		}
		// Live: the latest step and the beat.
		expect(parts("run:r-audit")).toMatchObject({ title: "Audit side-pane thread rows", line: "Read Dock.tsx", beats: true });
		expect(parts("run:r-pre")).toMatchObject({ line: "make precheck", beats: true });
		expect(parts("side:s1")).toMatchObject({ line: "Working · Yes, and keep it short.", beats: true });
		expect(parts("job:watch")).toMatchObject({ title: "Watch CI on release/0.49.0", line: "Running", beats: true });
		// State and result, no beat.
		expect(parts("side:h1")).toMatchObject({ line: "from Elvis · Three lines to approve.", beats: false });
		expect(parts("pair:lou~pork")).toMatchObject({ line: "Ended · Agreed the button order for task-71", beats: false });
		expect(parts("call:c1").line).toBe("Ended · 2 min");
	});

	test("A note row shows its words and \"from <source>\" and opens the thread it came from; a thread with notes shows a ✎ count", async () => {
		const roster = everyKind();
		asides = [
			aside("n1", today(0, 40), { text: "Linux and Windows are green on 2736d08d." }),
			aside("n2", today(0, 41), { text: "Run 7 is green too." }),
			aside("n3", today(0, 42), { by: "elvis", from: { thread: { kind: "side", key: "copy" } }, text: "Site copy is drafted; it needs the changelog lines." }),
			aside("n4", today(0, 43), { from: { inbound: "gh" }, sourceName: "hotline repo", text: "CI failed on main." }),
		];
		const { by: _by, ...outside } = asides[3]!;
		asides[3] = outside;
		const states: DockState[] = [];
		await dock(roster, states);
		expect(parts("note:n1")).toMatchObject({ title: "Linux and Windows are green on 2736d08d.", line: "from Watch CI on release/0.49.0 ›" });
		expect(parts("note:n3").line).toBe("from Elvis · work thread ›");
		expect(parts("note:n4").line).toBe("from hotline repo");
		// A note from outside has no thread to open.
		expect(row("note:n4")?.tagName).toBe("DIV");
		await act(async () => row("note:n1")!.click());
		expect(states).toEqual([{ open: { thread: { kind: "job", key: "watch" }, title: "Watch CI on release/0.49.0" } }]);
		// The job left two of them.
		expect(row("job:watch")?.querySelector(".list-notes")?.textContent).toBe("✎ 2");
		expect(row("side:s1")?.querySelector(".list-notes")).toBeNull();
	});

	test("The old groups (Subagents working, Threads, Ended subagents, Jobs, Closed) and the separate Noted page are gone; whatever opened Noted opens Threads filtered to Notes", async () => {
		const roster = everyKind();
		listed.push(
			summary({ thread: { kind: "run", key: "r-map" }, title: "Map browser stream latency", state: "closed", end: "done", outcome: "Input to picture is about 26 ms." }),
			summary({ thread: { kind: "side", key: "s2" }, title: "Server installer rerun", state: "closed", end: "agent", outcome: "Both fixes are live." }),
		);
		await dock(roster);
		const text = container.textContent ?? "";
		for (const gone of ["Subagents working", "Ended subagents", "Jobs", "Closed1", "Noted"]) expect(text).not.toContain(gone);
		expect(container.querySelectorAll(".dock section")).toHaveLength(0);
		// Ended ones are rows like any other, never folded away under Closed.
		expect(row("run:r-map")).not.toBeNull();
		expect(row("side:s2")).not.toBeNull();

		// The teammate menu's Notes: the same list, narrowed to the notes.
		const { teammateActions } = await import("../src/components/Conversation");
		const done: string[] = [];
		const actions = teammateActions({
			inspectorOpen: false, chapterBusy: false, unseen: 3,
			onToggleInspector: () => {}, onStartChapter: () => {}, onOpenThreadList: () => {},
			onOpenNotes: () => done.push("notes"), onDelete: () => {},
		});
		const notes = actions.find((one) => one.kind === "item" && one.id === "notes");
		expect(notes).toMatchObject({ text: "Notes", detail: "3 new" });
		expect(actions.some((one) => one.kind === "item" && one.text === "Noted")).toBe(false);
		await act(async () => rememberFilter("pork", "notes"));
		expect(chip("Notes")?.getAttribute("aria-pressed")).toBe("true");
		expect(rowIds()).toEqual(["note:n1"]);
		await act(async () => chip("All")!.click());
		expect(rowIds().length).toBeGreaterThan(1);
	});

	test("shown with notes in it, the list marks them read for the person", async () => {
		const roster = everyKind();
		asides = [aside("n1", today(0, 40)), aside("n0", today(0, 20), { seen: today(0, 25) })];
		await dock(roster);
		expect(sent.filter((one) => one.cmd === "aside.seen").map((one) => one.params)).toEqual([{ personaId: "pork" }]);
	});
});
