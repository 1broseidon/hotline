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

/**
 * A moment today, in order: `today(0, 30)` is thirty seconds after
 * `today(0, 0)`, both a moment past midnight, so a run just after midnight
 * still has them all today and in the order given.
 */
const today = (hours: number, minutes = 0) => {
	const midnight = new Date();
	midnight.setHours(0, 0, 0, 0);
	return Math.min(midnight.getTime() + (hours * 60 + minutes) * 1_000, Date.now() - 1);
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
let dom: import("happy-dom").Window;
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
	dom = installed.dom;
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

const DAY = 86_400_000;
/** A day's worth: one waiting, two running, today's, and what is earlier or routine. */
function aDay(personaId = "pork") {
	const owned = (row: Partial<ThreadSummary> & Pick<ThreadSummary, "thread">) => summary({ personaId, ...row });
	listed = [
		owned({ thread: { kind: "side", key: "w1" }, title: "Check the 0.49 site copy", waiting: true, updatedAt: today(0, 58) }),
		owned({ thread: { kind: "run", key: "r-audit" }, title: "Audit side-pane thread rows", working: true, updatedAt: today(0, 50) }),
		owned({ thread: { kind: "job", key: "watch" }, title: "Watch CI", working: true }),
		owned({ thread: { kind: "pair", key: `lou~${personaId}` }, title: "With Lou", state: "closed", end: "done", outcome: "Agreed the button order", updatedAt: today(0, 12) }),
		owned({ thread: { kind: "run", key: "r-map" }, title: "Map browser stream latency", state: "closed", end: "done", outcome: "Input to picture is about 26 ms.", startedAt: today(0, 4) - 180_000, updatedAt: today(0, 4) }),
		owned({ thread: { kind: "run", key: "r-pre" }, title: "make precheck", state: "closed", end: "done", outcome: "All 8 legs pass", updatedAt: today(0, 20), runKind: "shell" }),
		owned({ thread: { kind: "job", key: "sweep" }, title: "Sweep the inbox", state: "parked", updatedAt: today(0, 15), posted: false }),
		owned({ thread: { kind: "side", key: "old" }, title: "Server installer rerun", state: "closed", end: "agent", outcome: "Both fixes are live.", updatedAt: Date.now() - 3 * DAY }),
	];
	jobs = [job("watch", { personaId, name: "Watch CI on release/0.49.0" }), job("sweep", { personaId, name: "Sweep the inbox" })];
	asides = [
		aside("unread", today(0, 30), { to: personaId, text: "Linux and Windows are green." }),
		aside("read", today(0, 25), { to: personaId, text: "Site copy is drafted.", seen: today(0, 26) }),
		aside("friday", Date.now() - 4 * DAY, { to: personaId, text: "Disk is at 610 GB free." }),
	];
	const sides = [{ sideId: "w1", title: "Check the 0.49 site copy", startedAt: 1, working: false }];
	return [entry(persona(personaId, "Ada"), { subagents: [audit], workingJobs: ["watch"], sides }), entry(lou)];
}
const headings = () => [...container.querySelectorAll(".list-group, .list-fold")].map((one) => [...one.children].map((part) => part.textContent));
/** The rows under each heading, in order. */
const sections = () => {
	const out: Record<string, (string | null)[]> = {};
	let at = "";
	for (const one of container.querySelectorAll(".list-group, .list-fold, .list-row")) {
		if (!one.classList.contains("list-row")) {
			at = one.firstElementChild?.textContent ?? "";
			out[at] = [];
		} else out[at]?.push(one.getAttribute("data-thread"));
	}
	return out;
};

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
			"note:n1": "✎ Aside",
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

	test("The old groups (Subagents working, Threads, Ended subagents, Jobs, Closed) and the separate Noted page are gone; whatever opened Noted opens Threads filtered to what came from teammates", async () => {
		const roster = everyKind();
		listed.push(
			summary({ thread: { kind: "run", key: "r-map" }, title: "Map browser stream latency", state: "closed", end: "done", outcome: "Input to picture is about 26 ms." }),
			summary({ thread: { kind: "side", key: "s2" }, title: "Server installer rerun", state: "closed", end: "agent", outcome: "Both fixes are live." }),
		);
		await dock(roster);
		const headings = [...container.querySelectorAll(".list-group, .list-fold, h3")].map((one) => one.firstElementChild?.textContent ?? one.textContent);
		for (const gone of ["Subagents working", "Threads", "Ended subagents", "Jobs", "Closed"]) expect(headings).not.toContain(gone);
		expect(container.textContent).not.toContain("Noted");
		expect(container.querySelectorAll(".dock section, .noted-row")).toHaveLength(0);
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
		expect(notes).toMatchObject({ text: "Asides", detail: "3 new" });
		expect(actions.some((one) => one.kind === "item" && one.text === "Noted")).toBe(false);
		await act(async () => rememberFilter("pork", "teammates"));
		expect(chip("From teammates")?.getAttribute("aria-pressed")).toBe("true");
		// What other teammates sent: the asides they left and the work they handed over.
		expect(rowIds()).toEqual(expect.arrayContaining(["note:n1", "side:h1"]));
		expect(rowIds().every((id) => id?.startsWith("note:") || id === "side:h1")).toBe(true);
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

describe("task-401 the Active default", () => {
	test("Default filter Active groups Needs you, Live, Today, in that order", async () => {
		// A teammate whose list has never been narrowed opens on Active.
		await dock(aDay("ada"));
		expect(chip("Active")?.getAttribute("aria-pressed")).toBe("true");
		expect(headings().map((one) => one[0])).toEqual(["Needs you", "Live", "Today", "Earlier and routine · 5"]);
		expect(sections()).toEqual({
			"Needs you": ["side:w1"],
			Live: ["run:r-audit", "job:watch"],
			Today: ["note:unread", "pair:lou~ada", "run:r-map"],
			// Folded: the rest wait under one row.
			"Earlier and routine · 5": [],
		});
	});

	test("Earlier than today, routine items (a finished command, a job run that posted nothing) and read notes fold into one \"Earlier and routine · N\" row that expands in place", async () => {
		await dock(aDay());
		await act(async () => rememberFilter("pork", "active"));
		const fold = container.querySelector<HTMLButtonElement>(".list-fold")!;
		expect(fold.getAttribute("aria-expanded")).toBe("false");
		expect([...fold.children].map((part) => part.textContent)).toEqual(["Earlier and routine · 5", "Show"]);
		for (const folded of ["run:r-pre", "job:sweep", "note:read", "side:old", "note:friday"]) expect(row(folded)).toBeNull();
		await act(async () => fold.click());
		expect(fold.getAttribute("aria-expanded")).toBe("true");
		expect(fold.lastElementChild?.textContent).toBe("Hide");
		// In place: under the fold, newest first, and nothing above it moved.
		expect(sections()["Earlier and routine · 5"]).toEqual(["note:read", "run:r-pre", "job:sweep", "side:old", "note:friday"]);
		expect(parts("run:r-pre").mark).toBe("$");
		expect(parts("run:r-pre").line).toStartWith("Done · All 8 legs pass · ");
		expect(sections()["Today"]).toEqual(["note:unread", "pair:lou~pork", "run:r-map"]);
		// Under All nothing is folded: routine and read today are Today's, the rest Earlier.
		await act(async () => chip("All")!.click());
		expect(container.querySelector(".list-fold")).toBeNull();
		expect(sections()["Earlier"]).toEqual(["side:old", "note:friday"]);
		expect(sections()["Today"]).toContain("run:r-pre");
	});

	test("An ended subagent in Today shows its result line", async () => {
		await dock(aDay());
		await act(async () => rememberFilter("pork", "active"));
		expect(sections()["Today"]).toContain("run:r-map");
		expect(parts("run:r-map")).toMatchObject({ mark: "◆", line: "Reported · Input to picture is about 26 ms. · 3:00", beats: false });
	});

	test("Counts show only on Needs you and Live", async () => {
		await dock(aDay());
		await act(async () => rememberFilter("pork", "active"));
		expect(headings().slice(0, 3)).toEqual([["Needs you", "1"], ["Live", "2"], ["Today"]]);
		await act(async () => chip("All")!.click());
		expect(headings()).toEqual([["Needs you", "1"], ["Live", "2"], ["Today"], ["Earlier"]]);
	});
});

describe("task-402 filters and search", () => {
	const search = () => container.querySelector<HTMLInputElement>('input[aria-label="Search threads and asides"]')!;
	async function type(text: string) {
		await act(async () => {
			Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, "value")!.set!.call(search(), text);
			search().dispatchEvent(new dom.Event("input", { bubbles: true }) as unknown as Event);
		});
	}
	const span = (word: string) => [...container.querySelectorAll<HTMLButtonElement>('.list-span [role="tab"]')].find((one) => one.textContent === word);

	test("Chips: Active, All, Needs you, Live, Threads, Subagents, Jobs, Calls, Pairs, Notes. A kind chip shows every thread of that kind, live and ended, with a Live / Ended switch inside it", async () => {
		await dock(aDay());
		const words = [...container.querySelectorAll(".list-chips button")].map((one) => one.firstChild?.textContent);
		expect(words).toEqual(["Active", "All", "Needs you", "Live", "Threads", "Subagents", "Jobs", "Calls", "Pairs", "From teammates"]);
		// Only Needs you and Live are counted.
		expect(chip("Needs you")?.querySelector(".list-count")?.textContent).toBe("1");
		expect(chip("Live")?.querySelector(".list-count")?.textContent).toBe("2");
		expect(container.querySelectorAll(".list-count")).toHaveLength(2);

		// No switch outside a kind's chip.
		expect(container.querySelector(".list-span")).toBeNull();
		await act(async () => chip("Subagents")!.click());
		expect(rowIds()).toEqual(["run:r-audit", "run:r-map"]);
		expect(span("All")?.getAttribute("aria-selected")).toBe("true");
		await act(async () => span("Live")!.click());
		expect(rowIds()).toEqual(["run:r-audit"]);
		await act(async () => span("Ended")!.click());
		expect(rowIds()).toEqual(["run:r-map"]);

		// Threads are what is worked in: work threads, handoffs, commands.
		await act(async () => chip("Threads")!.click());
		expect(span("All")?.getAttribute("aria-selected")).toBe("true");
		expect(rowIds()).toEqual(["side:w1", "run:r-pre", "side:old"]);
		await act(async () => chip("Jobs")!.click());
		expect(rowIds()).toEqual(["job:watch", "job:sweep"]);
		await act(async () => span("Ended")!.click());
		expect(container.textContent).toContain("Nothing here.");
		await act(async () => chip("Pairs")!.click());
		expect(rowIds()).toEqual([`pair:lou~pork`]);
		await act(async () => chip("Calls")!.click());
		expect(rowIds()).toEqual([]);
		await act(async () => chip("Needs you")!.click());
		expect(rowIds()).toEqual(["side:w1"]);
		await act(async () => chip("Live")!.click());
		expect(rowIds()).toEqual(["run:r-audit", "job:watch"]);
		// Notes are not threads: no switch.
		await act(async () => chip("From teammates")!.click());
		expect(container.querySelector(".list-span")).toBeNull();
		expect(rowIds()).toEqual(["note:unread", "note:read", "note:friday"]);
	});

	test("Search matches titles, latest steps, results and note words across everything, earlier items included", async () => {
		await dock(aDay());
		await act(async () => rememberFilter("pork", "active"));
		expect(row("side:old")).toBeNull();
		// A title, earlier than today and folded away under Active.
		await type("installer");
		expect(rowIds()).toEqual(["side:old"]);
		expect(container.querySelector(".list-fold")).toBeNull();
		// A latest step.
		await type("read dock");
		expect(rowIds()).toEqual(["run:r-audit"]);
		// A result.
		await type("26 ms");
		expect(rowIds()).toEqual(["run:r-map"]);
		// A note's words, from four days ago.
		await type("610 GB");
		expect(rowIds()).toEqual(["note:friday"]);
		await type("nothing like this");
		expect(container.textContent).toContain("No matches.");
		// Escape clears the search, and the list is Active's again.
		await act(async () => search().dispatchEvent(new dom.KeyboardEvent("keydown", { key: "Escape", bubbles: true }) as unknown as Event));
		expect(search().value).toBe("");
		expect(container.querySelector(".list-fold")).not.toBeNull();
		// Under a kind's chip, it searches that kind.
		await act(async () => chip("Jobs")!.click());
		await type("inbox");
		expect(rowIds()).toEqual(["job:sweep"]);
		await type("installer");
		expect(rowIds()).toEqual([]);
	});

	test("The filter last used is kept per teammate across opening and closing the pane and restarts", async () => {
		const roster = aDay();
		await dock(roster);
		await act(async () => chip("Jobs")!.click());
		// Closed and opened again.
		await act(async () => root.render(<div />));
		await dock(roster);
		expect(chip("Jobs")?.getAttribute("aria-pressed")).toBe("true");
		// Another teammate keeps its own: Lou has never been narrowed.
		localStorage.removeItem("hotline.threads.filter");
		await act(async () => rememberFilter("pork", "jobs"));
		await dock([entry(lou), ...roster]);
		expect(chip("Active")?.getAttribute("aria-pressed")).toBe("true");
		// A restart: nothing but what the window stored, read afresh.
		expect(JSON.parse(localStorage.getItem("hotline.threads.filter") ?? "{}")).toEqual({ pork: "jobs" });
		localStorage.setItem("hotline.threads.filter", JSON.stringify({ pork: "calls", lou: "teammates" }));
		await act(async () => root.render(<div />));
		await dock(roster);
		expect(chip("Calls")?.getAttribute("aria-pressed")).toBe("true");
		// Something unreadable there opens on Active.
		localStorage.setItem("hotline.threads.filter", "{not json");
		await act(async () => root.render(<div />));
		await dock(roster);
		expect(chip("Active")?.getAttribute("aria-pressed")).toBe("true");
	});

	test("Whatever data a row needs and the list lacks (a finished run's kind, whether a job run posted, a note's read state) is added where it belongs", async () => {
		const { listItems } = await import("../src/threadList");
		const { rowOfLink } = await import("../src/dock");
		const base = { runs: [], jobs: [], asides: [], listedFor: "pork", nameOf: () => undefined, personName: "", now: Date.now() };
		// A finished run the roster has let go of: its kind is the list's word (`runKind`), or its link's before the list is read again.
		const ended = summary({ thread: { kind: "run", key: "cmd" }, title: "make precheck", state: "closed", end: "done", runKind: "shell" });
		const linked = rowOfLink({ kind: "link", id: "link:run:cmd2", ts: 1, thread: "cmd2", threadKind: "run", title: "cargo build", state: "closed", end: "done", runKind: "shell" }, "pork");
		const kinds = listItems({ ...base, rows: [ended, linked, summary({ thread: { kind: "run", key: "sub" }, state: "closed", end: "done" })] }).map((item) => [item.id, item.kind, item.routine]);
		expect(kinds).toEqual([
			["run:cmd", "command", true],
			["run:sub", "subagent", false],
			["run:cmd2", "command", true],
		]);
		// A job whose latest run posted nothing is routine; one that posted, or is running, is not.
		const jobRows = (posted: boolean, working = false) =>
			listItems({ ...base, jobs: [job("j")], rows: [summary({ thread: { kind: "job", key: "j" }, posted, working })] })[0]!;
		expect(jobRows(false).routine).toBe(true);
		expect(jobRows(true).routine).toBe(false);
		expect(jobRows(false, true).routine).toBe(false);
		// A note is read once the person has seen it (`aside.seen`).
		const notes = listItems({ ...base, rows: [], asides: [aside("a", 2, { seen: 3 }), aside("b", 1)] });
		expect(notes.map((item) => [item.id, item.read])).toEqual([["note:a", true], ["note:b", false]]);
	});
});

describe("task-403 needs-you actions", () => {
	const question: TranscriptEvent = { kind: "human_action", id: "q", ts: 1, actionId: "q", reason: "Ship the three lines as drafted?", question: true, status: "pending" };
	const permission: TranscriptEvent = {
		kind: "permission", id: "perm", ts: 1, requestId: "req", title: "docker image prune -a",
		options: [
			{ optionId: "once", name: "Allow once", kind: "allow_once" },
			{ optionId: "no", name: "Deny", kind: "reject_once" },
		],
	};
	const passkey: TranscriptEvent = { kind: "passkey_ask", id: "pk", ts: 1, askId: "pk", name: "GITHUB", rpId: "github.com", origin: "https://github.com", rpName: "GitHub", status: "pending" };
	const credential: TranscriptEvent = { kind: "credential_ask", id: "cred", ts: 1, actionId: "cred", site: "https://porkbun.com/", reason: "To list your domains.", credentialKind: "login", name: "PORKBUN", status: "pending", expiresAt: Date.now() + 600_000, existing: [], armed: false };
	const handoffAsk: TranscriptEvent = { kind: "human_action", id: "h", ts: 1, actionId: "h", reason: "Which of the three lines goes first?", status: "pending" };

	/** Five threads waiting, each on one kind of card. */
	function waiting() {
		const waits = (key: string, kind: "side" | "run", title: string, extra: Partial<ThreadSummary> = {}) =>
			summary({ thread: { kind, key }, title, waiting: true, ...extra });
		listed = [
			waits("w-q", "side", "Release notes for 0.49", { updatedAt: today(0, 5) }),
			waits("r-perm", "run", "Prune the images", { updatedAt: today(0, 4) }),
			waits("w-pk", "side", "Make a GitHub passkey", { updatedAt: today(0, 3) }),
			waits("w-cred", "side", "List the domains", { updatedAt: today(0, 2) }),
			waits("w-hand", "side", "Check the 0.49 site copy", { updatedAt: today(0, 1), opener: { personaId: "elvis", name: "Elvis" } }),
		];
		threadLines = { "w-q": [question], "r-perm": [permission], "w-pk": [passkey], "w-cred": [credential], "w-hand": [handoffAsk] };
		const sides = ["w-q", "w-pk", "w-cred", "w-hand"].map((sideId) => ({ sideId, title: sideId, startedAt: 1, working: false }));
		return [entry(pork, { sides, subagents: [{ runId: "r-perm", title: "Prune the images", startedAt: 1 }] }), entry(elvis)];
	}
	const keys = (id: string) => [...(row(id)?.closest(".list-needs")?.querySelectorAll<HTMLButtonElement>(".list-keys button") ?? [])];
	const words = (id: string) => keys(id).map((one) => one.textContent);
	const press = async (id: string, word: string) => {
		const key = keys(id).find((one) => one.textContent === word);
		if (key === undefined) throw new Error(`No ${word} on ${id}: ${words(id)}`);
		await act(async () => key.click());
	};

	test("A row in Needs you shows the waiting card's question in its line and its primary answer keys (the card's own: Yes / No, Approve, Answer…) on the row; pressing one answers the card exactly as the card in the thread would", async () => {
		await dock(waiting());
		await settle();
		expect(sections()["Needs you"]).toEqual(["side:w-q", "run:r-perm", "side:w-pk", "side:w-cred", "side:w-hand"]);
		// The question is the line; the card's own answers are the keys.
		expect(parts("side:w-q").line).toBe("Ship the three lines as drafted?");
		expect(words("side:w-q")).toEqual(["Yes", "No"]);
		expect(parts("run:r-perm").line).toBe("docker image prune -a");
		expect(words("run:r-perm")).toEqual(["Allow once", "Deny"]);
		expect(parts("side:w-pk").line).toBe("Passkey for GitHub");
		expect(words("side:w-pk")).toEqual(["Approve", "Deny"]);

		// Each answers as its card does: the same command, the same words.
		await press("side:w-q", "Yes");
		await press("run:r-perm", "Allow once");
		await press("side:w-pk", "Approve");
		const answered = sent.filter((one) => one.cmd === "thread.answer" || one.cmd === "secrets.passkey.answer");
		expect(answered).toEqual([
			{ cmd: "thread.answer", params: { thread: { kind: "side", key: "w-q" }, answer: { kind: "human", actionId: "q", status: "done" } } },
			{ cmd: "thread.answer", params: { thread: { kind: "run", key: "r-perm" }, answer: { kind: "permission", requestId: "req", optionId: "once" } } },
			{ cmd: "secrets.passkey.answer", params: { personaId: "pork", askId: "pk", approved: true } },
		]);
		// One answer: the row's keys wait for it, and so does the card in the thread, which is the same card.
		expect(keys("side:w-q").every((one) => one.disabled)).toBe(true);
		const { HumanAction } = await import("../src/components/transcript/Asks");
		const card = document.createElement("div");
		document.body.append(card);
		const { createRoot } = await import("react-dom/client");
		const cardRoot = createRoot(card);
		await act(async () => cardRoot.render(<HumanAction thread={{ kind: "side", key: "w-q" }} event={question as Extract<TranscriptEvent, { kind: "human_action" }>} who={{ id: "pork", name: "Pork Chop" }} />));
		const yes = [...card.querySelectorAll<HTMLButtonElement>("button")].find((one) => one.textContent === "Yes");
		expect(yes?.disabled).toBe(true);
		await act(async () => cardRoot.unmount());
	});

	test("Where the card's answer needs more than a key (text, a credential), the row shows Open instead of a fake key", async () => {
		const states: DockState[] = [];
		await dock(waiting(), states);
		await settle();
		// A sign-in takes a credential; a handoff's question takes written words.
		expect(parts("side:w-cred").line).toBe("Sign in to https://porkbun.com/");
		expect(words("side:w-cred")).toEqual(["Open"]);
		expect(parts("side:w-hand").line).toBe("Which of the three lines goes first?");
		expect(words("side:w-hand")).toEqual(["Open"]);
		await press("side:w-cred", "Open");
		expect(states).toEqual([{ open: { thread: { kind: "side", key: "w-cred" }, title: "List the domains" } }]);
		expect(sent.some((one) => one.cmd === "thread.answer")).toBe(false);
	});
});
