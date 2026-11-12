import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { RosterEntry, RunningSide, RunningSubagent, ScheduleEntry, ThreadSummary } from "../src/generated/contract";
import type { LiveThread } from "../src/liveThreads";
import { installDom } from "./dom";

// task-437: the desk composer counts the threads running in the session, the
// Threads list's Live group ("2 threads ›"; George, 2026-11-11: the count is
// what runs now, so "running" is true and it agrees with Live), lists them in
// a panel that grows the pill up, and the per-subagent chips that stood on
// the pill's corner are gone.

const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });
const real = await import("../src/voice/transcription");
mock.module("../src/voice/transcription", () => ({ ...real, hearsOnThisMac: async () => true }));
const { liveThreads, threadsLabel, threadsWords } = await import("../src/liveThreads");
const { grouped, listItems } = await import("../src/threadList");
const { withTurns } = await import("../src/dock");

const NOW = 1_000_000_000;
const persona = (id: string, name: string) => ({
	id, name, goal: "", backendId: "hotline", cwd: `/work/${id}`,
	mcpPolicy: { mode: "none" as const, serverIds: [] }, skillPolicy: { mode: "none" as const, names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 0, updatedAt: 0,
});
const session = (personaId: string, state: "ready" | "thinking" = "ready") => ({
	personaId, state, contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
	capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
});
type Going = { sides?: RunningSide[]; subagents?: RunningSubagent[]; workingJobs?: string[]; workingCalls?: string[]; workingPairs?: string[] };
const entry = (going: Going = {}, state: "ready" | "thinking" = "ready", id = "ada", name = "Ada"): RosterEntry => ({
	persona: persona(id, name), session: session(id, state), waiting: false, drawing: false,
	...Object.fromEntries(Object.entries(going).filter(([, list]) => list.length > 0)),
});

const audit: RunningSubagent = { runId: "r-audit", title: "Audit side-pane thread rows", startedAt: NOW - 134_000, lastStep: "Read Dock.tsx", steps: 4 };
const fresh: RunningSubagent = { runId: "r-fresh", title: "Check the model list", startedAt: NOW - 5_000 };
const crawl: RunningSubagent = { runId: "r-crawl", title: "Crawl 12 sites", startedAt: NOW - 40_000, kind: "shell", command: "python3 crawl.py" };
const notes: RunningSide = { sideId: "s-notes", title: "Release notes", startedAt: NOW - 580_000, working: true };
const installer: RunningSide = { sideId: "s-hand", title: "Check the installer", startedAt: NOW - 1_260_000, working: true };
const idle: RunningSide = { sideId: "s-idle", title: "Pick a font", startedAt: NOW - 3_000_000, working: false };
const asking: RunningSide = { sideId: "s-ask", title: "Rotate the keys", startedAt: NOW - 90_000, working: true };
const digest: ScheduleEntry = { id: "j-digest", personaId: "ada", name: "Morning digest", kind: "loop", prompt: "Digest", every: 86_400_000, nextAt: NOW + 3_600_000 };

/** A row of `thread.list` for the teammate. */
const listed = (kind: ThreadSummary["thread"]["kind"], key: string, more: Partial<ThreadSummary> = {}): ThreadSummary => ({
	thread: { kind, key }, personaId: "ada", state: "live", startedAt: NOW - 600_000, updatedAt: NOW - 1_000, working: false, waiting: false, ...more,
});
/** What `thread.list` says for the sides and runs given, as the desk lists them. */
const listOf = (sides: RunningSide[], runs: RunningSubagent[]): ThreadSummary[] => [
	...sides.map((side) => listed("side", side.sideId, { title: side.title, startedAt: side.startedAt, ...(side === installer ? { opener: { personaId: "mack", name: "Mack" } } : {}), ...(side === asking ? { waiting: true } : {}) })),
	...runs.map((run) => listed("run", run.runId, { title: run.title, startedAt: run.startedAt, ...(run.kind === "shell" ? { runKind: "shell" as const } : {}) })),
];
const names: Record<string, string> = { ada: "Ada", lou: "Lou", mack: "Mack" };
const liveOf = (row: RosterEntry, rows: ThreadSummary[], jobs: ScheduleEntry[] = []) =>
	liveThreads({ listed: rows, roster: [row], entry: row, jobs, nameOf: (id) => names[id], now: NOW });
/** The Threads list's Live group, as the pane draws it from the same rows (ThreadList: listItems, then grouped). */
const liveGroup = (row: RosterEntry, rows: ThreadSummary[], jobs: ScheduleEntry[] = []) =>
	grouped(listItems({ rows: withTurns(rows, [row], "ada"), runs: row.subagents ?? [], jobs, asides: [], listedFor: "ada", nameOf: (id) => names[id], personName: "", now: NOW }), NOW, true).live;

/** Everything a teammate can have going: what runs, what rests, what waits and what has ended. */
function busyAda() {
	const row = entry({ sides: [notes, installer, idle, asking], subagents: [audit, fresh, crawl], workingJobs: ["j-digest"], workingCalls: ["c-1"], workingPairs: ["ada~lou"] }, "thinking");
	const rows = [
		...listOf([notes, installer, idle, asking], [audit, fresh, crawl]),
		listed("call", "c-1", { title: "Standup", startedAt: NOW - 300_000 }),
		listed("job", "j-digest"),
		listed("pair", "ada~lou", { working: true }),
		listed("pair", "ada~mack"),
		listed("run", "r-old", { title: "Earlier audit", state: "closed", end: "done" }),
	];
	return { row, rows, jobs: [digest] };
}

describe("task-437 the count", () => {
	test("reads 1, 2 and 9+ the phone's way, and says how many run to a screen reader", () => {
		expect(threadsWords(1)).toBe("1 thread");
		expect(threadsWords(2)).toBe("2 threads");
		expect(threadsWords(9)).toBe("9 threads");
		expect(threadsWords(12)).toBe("9+ threads");
		expect(threadsLabel(1)).toBe("1 thread running");
		expect(threadsLabel(12)).toBe("12 threads running");
	});

	test("is 0 with nothing running: an open work thread at rest and a thread that has ended are not counted", () => {
		const row = entry({ sides: [idle] });
		const rows = [...listOf([idle], []), listed("run", "r-old", { title: "Earlier audit", state: "closed", end: "done" })];
		expect(liveOf(row, rows)).toEqual([]);
		expect(liveOf(entry(), [])).toEqual([]);
	});

	test("says the same number as the Threads list's Live group, row for row", () => {
		for (const [row, rows, jobs] of [
			[entry({ subagents: [audit] }), listOf([], [audit]), []],
			[entry({ sides: [installer, idle], subagents: [audit] }), listOf([installer, idle], [audit]), []],
			[busyAda().row, busyAda().rows, busyAda().jobs],
		] as const) {
			const live = liveOf(row, [...rows], [...jobs]);
			const group = liveGroup(row, [...rows], [...jobs]);
			expect(live.length).toBe(group.length);
			expect(live.map((one) => one.id)).toEqual(group.map((item) => item.id));
		}
		expect(liveOf(busyAda().row, busyAda().rows, busyAda().jobs).length).toBe(8);
	});

	test("lists only what runs now, each with its kind, what it is doing and how long it has run", () => {
		const { row, rows, jobs } = busyAda();
		const live = liveOf(row, rows, jobs);
		const byId = Object.fromEntries(live.map((one) => [one.id, one]));
		expect(Object.fromEntries(live.map((one) => [one.id, one.line]))).toEqual({
			"side:s-notes": "Work thread · Working",
			"side:s-hand": "Handoff from Mack · Working",
			"run:r-audit": "Helper · Read Dock.tsx",
			"run:r-fresh": "Helper · Starting",
			"run:r-crawl": "Command · python3 crawl.py",
			"call:c-1": "Call · Working",
			"job:j-digest": "Job · Running",
			"pair:ada~lou": "Pair · Working",
		});
		// At rest, waiting on the person (Needs you), or ended: not here.
		expect(live.map((one) => one.id)).not.toContain("side:s-idle");
		expect(live.map((one) => one.id)).not.toContain("side:s-ask");
		expect(live.map((one) => one.id)).not.toContain("pair:ada~mack");
		expect(live.map((one) => one.id)).not.toContain("run:r-old");
		// How long: since each started; a pair or a job, whose thread outlives its turns, has none.
		expect(byId["run:r-audit"]?.since).toBe(audit.startedAt);
		expect(byId["side:s-hand"]?.since).toBe(installer.startedAt);
		expect(byId["pair:ada~lou"]?.since).toBeUndefined();
		expect(byId["job:j-digest"]?.since).toBeUndefined();
		// Each opens as the list's row does.
		expect(byId["run:r-audit"]?.open).toEqual({ thread: { kind: "run", key: "r-audit" }, title: "Audit side-pane thread rows" });
		expect(byId["job:j-digest"]?.open).toEqual({ thread: { kind: "job", key: "j-digest" }, title: "Morning digest" });
	});

	test("a pair runs while the roster says one of its two answers the other, whatever an older read of the list said", () => {
		const rows = [listed("pair", "ada~lou", { withPersonaId: "lou", preview: "Is the deploy paused?" })];
		expect(liveOf(entry(), rows)).toEqual([]);
		const live = liveOf(entry({ workingPairs: ["ada~lou"] }), rows);
		expect(live.map((one) => [one.id, one.line])).toEqual([["pair:ada~lou", "Pair · Working · Is the deploy paused?"]]);
		// The list read mid-answer, the answer since in: the roster's word stands.
		expect(liveOf(entry(), [{ ...rows[0]!, state: "live", working: true }])).toEqual([]);
	});

	test("a work thread joins the count when a turn of it starts and leaves it when the turn ends", () => {
		const rows = listOf([idle], []);
		expect(liveOf(entry({ sides: [idle] }), rows)).toEqual([]);
		expect(liveOf(entry({ sides: [{ ...idle, working: true }] }), rows).map((one) => one.line)).toEqual(["Work thread · Working"]);
	});
});

let act: typeof import("react").act;
let root: Root | null;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

describe("task-437 the control and its panel", () => {
	beforeEach(async () => {
		({ teardown } = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} }));
		act = (await import("react")).act;
		const { createRoot } = await import("react-dom/client");
		container = document.createElement("div");
		document.body.append(container);
		root = createRoot(container);
	});
	afterEach(async () => {
		await act(async () => root?.unmount());
		root = null;
		await teardown();
	});

	const opened: LiveThread[] = [];
	const cancels: number[] = [];
	const alls: number[] = [];
	async function composer(rows: LiveThread[], state: "ready" | "thinking" = "ready", fresh?: number) {
		const { Composer } = await import("../src/components/Composer");
		await act(async () =>
			root!.render(
				<div>
					<p className="away">The conversation</p>
					<Composer
						personaId="ada"
						name="Ada"
						state={state}
						replyQuote={null}
						onSend={() => {}}
						onCancel={() => cancels.push(1)}
						onClearReply={() => {}}
						threads={{ rows, fresh, isOpen: (one) => one.id === "run:r-audit", onOpen: (one) => opened.push(one), onAll: () => alls.push(1) }}
					/>
				</div>,
			),
		);
	}
	const key = () => container.querySelector<HTMLButtonElement>("button.composer-threads");
	const panel = () => container.querySelector<HTMLElement>(".composer-threads-panel");
	const rows = () => [...container.querySelectorAll<HTMLButtonElement>(".composer-threads-panel .list-row[data-thread]")];
	const allRow = () => container.querySelector<HTMLButtonElement>(".composer-threads-panel .composer-threads-all");
	const field = () => container.querySelector("textarea")!;
	const press = async (target: Element) => act(async () => void target.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
	const down = async (target: Element) => act(async () => void target.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true })));
	const keyDown = async (target: Element, key: string) => act(async () => void target.dispatchEvent(new window.KeyboardEvent("keydown", { key, bubbles: true })));
	const live = (sides: RunningSide[], runs: RunningSubagent[]) => liveOf(entry({ sides, subagents: runs }), listOf(sides, runs));

	beforeEach(() => {
		opened.length = 0;
		cancels.length = 0;
		alls.length = 0;
	});

	test("is absent at zero, and reads 1 thread, 2 threads, 9+ threads at the bottom row's right as a button labelled how many run", async () => {
		await composer([]);
		expect(key()).toBeNull();
		// At rest with nothing live the pill stays one row.
		expect(container.querySelector(".composer")?.hasAttribute("data-work")).toBe(false);

		await composer(live([], [audit]));
		expect(key()?.textContent).toBe("1 thread");
		expect(key()?.getAttribute("aria-label")).toBe("1 thread running");
		expect(container.querySelector(".composer")?.hasAttribute("data-work")).toBe(true);

		await composer(live([installer], [audit]));
		expect(key()?.textContent).toBe("2 threads");
		expect(key()?.getAttribute("aria-label")).toBe("2 threads running");
		expect(key()?.title).toBe("Show threads");

		const many = Array.from({ length: 12 }, (_, at): RunningSubagent => ({ runId: `r${at}`, title: `Check ${at}`, startedAt: NOW }));
		await composer(live([], many));
		expect(key()?.textContent).toBe("9+ threads");
		expect(key()?.getAttribute("aria-label")).toBe("12 threads running");
	});

	test("is reached by Tab: a button after the field, before the keys", async () => {
		await composer(live([], [audit]));
		const focusable = [...container.querySelectorAll<HTMLElement>("button, textarea")].filter((one) => one.tabIndex >= 0);
		expect(key()?.tagName).toBe("BUTTON");
		expect(focusable.indexOf(key()!)).toBe(focusable.indexOf(field()) + 1);
		expect(focusable.at(-1)?.getAttribute("aria-label")).toBe("Dictate");
	});

	test("opens a panel of only the running threads, titled how many run, each row its kind, what it is doing and how long", async () => {
		await composer(live([installer, idle], [audit]));
		expect(panel()).toBeNull();
		await press(key()!);
		expect(key()?.getAttribute("aria-expanded")).toBe("true");
		expect(panel()?.getAttribute("aria-label")).toBe("2 threads running");
		expect(panel()?.querySelector("h3")?.textContent).toBe("2 threads running");
		expect(key()?.getAttribute("aria-controls")).toBe(panel()?.id);
		// The pill's first row: inside it, before the field.
		expect(panel()?.parentElement?.className).toBe("composer");
		expect(rows().map((row) => [row.querySelector(".list-title-text")?.textContent, row.querySelector(".list-line")?.textContent])).toEqual([
			["Check the installer", "Handoff from Mack · Working"],
			["Audit side-pane thread rows", "Helper · Read Dock.tsx"],
		]);
		// Each row beats: everything in it runs.
		expect(rows().every((row) => row.getAttribute("data-state") === "live" && row.querySelector(".dock-dot.beat") !== null)).toBe(true);
		const when = rows().map((row) => row.querySelector(".list-when")?.textContent ?? "");
		expect(when.every((one) => /^\d+:\d\d(:\d\d)?$/.test(one))).toBe(true);
		expect(rows()[1]?.getAttribute("aria-label")).toStartWith("Audit side-pane thread rows, Helper · Read Dock.tsx, ");
		// The one open in the pane says so, as the chip's pressed state did.
		expect(rows()[1]?.getAttribute("aria-current")).toBe("true");
		expect(rows()[0]?.hasAttribute("aria-current")).toBe(false);
	});

	test("a row opens its thread and the panel closes", async () => {
		await composer(live([installer], [audit]));
		await press(key()!);
		await press(rows()[1]!);
		expect(opened.map((one) => one.open)).toEqual([{ thread: { kind: "run", key: "r-audit" }, title: "Audit side-pane thread rows" }]);
		expect(panel()).toBeNull();
	});

	test("Escape closes it, before it reaches the turn, and focus returns to the control", async () => {
		await composer(live([], [audit]), "thinking");
		await press(key()!);
		rows()[0]!.focus();
		await keyDown(rows()[0]!, "Escape");
		expect(panel()).toBeNull();
		expect(cancels).toEqual([]);
		expect(document.activeElement).toBe(key());
		// Typing in the field under it, Escape closes it and leaves the field where the words go.
		await press(key()!);
		field().focus();
		await keyDown(field(), "Escape");
		expect(panel()).toBeNull();
		expect(cancels).toEqual([]);
		expect(document.activeElement).toBe(field());
		// With it closed, Escape is the turn's again.
		await keyDown(field(), "Escape");
		expect(cancels).toEqual([1]);
	});

	test("a press outside the composer closes it; the field under it stays in use", async () => {
		await composer(live([], [audit]));
		await press(key()!);
		await down(field());
		field().focus();
		expect(panel()).not.toBeNull();
		expect(document.activeElement).toBe(field());
		await down(container.querySelector(".away")!);
		expect(panel()).toBeNull();
	});

	test("the control pressed again closes it", async () => {
		await composer(live([], [audit]));
		await press(key()!);
		expect(panel()).not.toBeNull();
		await press(key()!);
		expect(panel()).toBeNull();
		expect(key()?.getAttribute("aria-expanded")).toBe("false");
		expect(key()?.title).toBe("Show threads");
	});

	test("opened from the keyboard it takes focus to its first row; arrows walk the rows and All threads, and down past the last is the control again", async () => {
		await composer(live([notes, installer], [audit]));
		key()!.focus();
		// Enter or Space on a button is a click with no pointer behind it.
		await act(async () => key()!.click());
		expect(document.activeElement).toBe(rows()[0]!);
		await keyDown(rows()[0]!, "ArrowDown");
		expect(document.activeElement).toBe(rows()[1]!);
		await keyDown(rows()[1]!, "End");
		expect(document.activeElement).toBe(allRow());
		await keyDown(allRow()!, "ArrowUp");
		expect(document.activeElement).toBe(rows()[2]!);
		await keyDown(rows()[2]!, "Home");
		expect(document.activeElement).toBe(rows()[0]!);
		await keyDown(rows()[0]!, "End");
		await keyDown(allRow()!, "ArrowDown");
		expect(document.activeElement).toBe(key());
		// From the control, up reaches the row nearest it.
		await keyDown(key()!, "ArrowUp");
		expect(document.activeElement).toBe(allRow());
	});

	test("its last row, All threads, opens the full list in the pane with how many things are new there, and the panel closes", async () => {
		// desk-threads, decision 5: idle threads are not in the panel; the list in the pane has them.
		await composer(live([installer], [audit]), "ready", 2);
		await press(key()!);
		expect(allRow()?.textContent).toBe("⋯All threads2 new");
		expect(allRow()?.getAttribute("aria-label")).toBe("All threads, 2 new");
		await press(allRow()!);
		expect(alls).toEqual([1]);
		expect(panel()).toBeNull();
		// With nothing new it says only what it is.
		await composer(live([installer], [audit]));
		await press(key()!);
		expect(allRow()?.getAttribute("aria-label")).toBe("All threads");
		expect(allRow()?.textContent).toBe("⋯All threads");
	});

	test("the work row pressed with it open closes it, and shows what the row holds rather than folding it", async () => {
		const { Composer } = await import("../src/components/Composer");
		const toggles: number[] = [];
		const render = (open: boolean) =>
			act(async () =>
				root!.render(
					<Composer
						personaId="ada"
						name="Ada"
						state="thinking"
						replyQuote={null}
						onSend={() => {}}
						onCancel={() => {}}
						onClearReply={() => {}}
						above={<div className="work-dock">The steps</div>}
						work={{ activity: { phase: "thinking", word: "Thinking" }, sleeping: false, open, onToggle: () => toggles.push(1) }}
						threads={{ rows: live([], [audit]), isOpen: () => false, onOpen: () => {}, onAll: () => {} }}
					/>,
				),
			);
		const row = () => container.querySelector<HTMLButtonElement>("button.composer-work")!;
		// Its steps open: the threads take the pill's first row while they are open, and the row gives it back.
		await render(true);
		await press(key()!);
		expect(container.querySelector(".work-dock")).toBeNull();
		await press(row());
		expect(panel()).toBeNull();
		expect(toggles).toEqual([]);
		expect(container.querySelector(".work-dock")?.textContent).toBe("The steps");
		// Folded, the row opens them as it always has, and the threads close.
		await render(false);
		await press(key()!);
		await press(row());
		expect(panel()).toBeNull();
		expect(toggles).toEqual([1]);
	});

	test("closes when the last live thread ends, and stays closed for the next", async () => {
		await composer(live([], [audit]));
		await press(key()!);
		await composer([]);
		expect(key()).toBeNull();
		await composer(live([], [fresh]));
		expect(panel()).toBeNull();
	});
});

describe("task-437 in the conversation", () => {
	// Its own teammate: the window keeps each thread's store by desk and id for the
	// process, so an "ada" another file opened would answer with that file's socket.
	// Its own teammate: the window keeps each thread's store by desk and id for the
	// process, so an "ada" another file opened would answer with that file's socket.
	// A handoff and a subagent and a command run; a work thread rests.
	/** Whether Nell's ask of Lou is being answered now: the core's word on her roster row and in her list. */
	let pairAnswering = false;
	const nell = (): RosterEntry => ({
		...entry({ sides: [installer, idle], subagents: [audit, crawl] }, "thinking", "nell", "Nell"),
		...(pairAnswering ? { workingPairs: ["lou~nell"] } : {}),
	});
	const nellsList = (): ThreadSummary[] => [
		...listOf([installer, idle], [audit, crawl]).map((row) => ({ ...row, personaId: "nell" })),
		// The thread between Nell and Lou, as `thread.list` says it: live only while an ask of it is answered.
		listed("pair", "lou~nell", { personaId: "nell", withPersonaId: "lou", withName: "Lou", state: pairAnswering ? "live" : "parked", working: pairAnswering, preview: "Is the nightly deploy paused?" }),
	];

	class FakeCore {
		static readonly OPEN = 1;
		/** The teammate's threads, as `thread.list` says them, have reached the window. */
		static listDelivered = false;
		/** How many times the window asked for the list. */
		static listReads = 0;
		/** The socket open now, and its roster subscription: where a roster row is pushed. */
		static open: FakeCore | null = null;
		static rosterSub: unknown = null;
		/** The core sends Nell's roster row again, as it does when her row changes. */
		static pushRoster() {
			FakeCore.open?.onmessage?.({ data: JSON.stringify({ sub: FakeCore.rosterSub, event: nell() }) });
		}
		readyState = 1;
		onopen: (() => void) | null = null;
		onmessage: ((message: { data: string }) => void) | null = null;
		onclose: (() => void) | null = null;
		onerror: (() => void) | null = null;
		constructor(readonly url: string) {
			FakeCore.open = this;
			setTimeout(() => this.onopen?.(), 0);
		}
		send(data: string) {
			const frame = JSON.parse(data) as Record<string, unknown>;
			const id = frame["id"];
			const reply = (answer: Record<string, unknown>) => setTimeout(() => this.onmessage?.({ data: JSON.stringify(answer) }), 0);
			if ("sub" in frame) {
				const target = JSON.stringify(frame["sub"]);
				if (target.includes('"roster"')) FakeCore.rosterSub = id;
				reply({ id, ok: true });
				reply({ sub: id, snapshot: target.includes('"roster"') ? [nell()] : [] });
				return;
			}
			if (frame["cmd"] === "client.hello") reply({ id, ok: true, result: { capabilities: ["threads", "threads2"] } });
			else if (frame["cmd"] === "thread.list") {
				FakeCore.listReads += 1;
				setTimeout(() => {
					this.onmessage?.({ data: JSON.stringify({ id, ok: true, result: nellsList() }) });
					FakeCore.listDelivered = true;
				}, 0);
			} else if (frame["cmd"] === "aside.list") reply({ id, ok: true, result: [] });
			else reply({ id, ok: false, error: "Not in this test." });
		}
		close() {
			if (this.readyState === 3) return;
			this.readyState = 3;
			this.onclose?.();
		}
	}

	beforeEach(async () => {
		FakeCore.listDelivered = false;
		FakeCore.listReads = 0;
		pairAnswering = false;
		const installed = installDom({ ResizeObserver: NoResize, WebSocket: FakeCore, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
		teardown = installed.teardown;
		// The pane puts focus back on an element when it closes.
		Object.assign(globalThis, { HTMLElement: installed.dom.HTMLElement });
		const { replaceDesks } = await import("../src/desks");
		replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "ws://core", token: "t" }]);
		act = (await import("react")).act;
		const { createRoot } = await import("react-dom/client");
		container = document.createElement("div");
		document.body.append(container);
		root = createRoot(container);
	});
	afterEach(async () => {
		await act(async () => root?.unmount());
		root = null;
		const { replaceDesks } = await import("../src/desks");
		replaceDesks([]);
		Reflect.deleteProperty(globalThis, "HTMLElement");
		await teardown();
	});

	async function settle() {
		for (let turn = 0; turn < 8; turn++) await act(async () => void (await new Promise((resolve) => setTimeout(resolve, 5))));
	}
	/** Until what the core sent has been drawn: the tape is asked for once the socket is open, after the roster. */
	async function until(drawn: () => boolean, what: string) {
		for (let turn = 0; turn < 400 && !drawn(); turn++) await act(async () => void (await new Promise((resolve) => setTimeout(resolve, 5))));
		if (!drawn()) throw new Error(`Never drawn: ${what}`);
	}

	test("the subagent chips are gone from the composer, and what each opened is a row of the panel that opens it in the pane", async () => {
		const { App } = await import("../src/App");
		await act(async () => root!.render(<App />));
		await settle();
		await act(async () => container.querySelector<HTMLButtonElement>('[data-teammate-row][aria-label^="Nell"]')!.click());
		// What runs is the Threads list's word, asked for once the socket is open, after the roster.
		await until(() => FakeCore.listDelivered, "the teammate's threads");
		await settle();
		// No chip stands on the pill's corner for the runs: nothing there at all, with no process listed.
		expect(container.querySelector(".composer-corner")).toBeNull();
		expect(container.querySelector('[aria-label^="Subagent working"], [aria-label^="Command running"], [aria-label$="subagents working"]')).toBeNull();
		const key = container.querySelector<HTMLButtonElement>("button.composer-threads");
		// The work thread at rest is not counted.
		expect(key?.textContent).toBe("3 threads");
		await act(async () => void key!.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		const rows = [...container.querySelectorAll<HTMLButtonElement>(".composer-threads-panel .list-row[data-thread]")];
		expect(rows.map((row) => row.querySelector(".list-line")?.textContent)).toEqual([
			"Handoff from Mack · Working",
			"Helper · Read Dock.tsx",
			"Command · python3 crawl.py",
		]);
		// The subagent's chip opened its run in the pane; its row does the same.
		await act(async () => void rows[1]!.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		await settle();
		expect(container.querySelector(".composer-threads-panel")).toBeNull();
		expect(container.querySelector(".dock .run-pane")?.getAttribute("aria-label")).toBe("Helper of Nell: Audit side-pane thread rows");
		// Open in the pane, its row says so.
		await act(async () => void key!.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		const again = [...container.querySelectorAll<HTMLButtonElement>(".composer-threads-panel .list-row[data-thread]")];
		expect(again.map((row) => row.getAttribute("aria-current"))).toEqual([null, "true", null]);
		// The command's chip opened its run too.
		await act(async () => void again[2]!.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		await settle();
		expect(container.querySelector(".dock .run-pane, .dock [aria-label^='Command']")).not.toBeNull();
	});

	test("says the same number as the Threads list's Live group beside it", async () => {
		const { App } = await import("../src/App");
		await act(async () => root!.render(<App />));
		await settle();
		await act(async () => container.querySelector<HTMLButtonElement>('[data-teammate-row][aria-label^="Nell"]')!.click());
		await until(() => FakeCore.listDelivered, "the teammate's threads");
		await settle();
		// The list, from the teammate's menu.
		await act(async () => container.querySelector<HTMLButtonElement>(".model-pill")!.click());
		const threads = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((item) => item.textContent?.startsWith("Threads"));
		await act(async () => threads!.click());
		await until(() => container.querySelector(".dock .list-group") !== null, "the Threads list");
		const live = [...container.querySelectorAll(".dock .list-group")].find((group) => group.textContent?.startsWith("Live"));
		const listed = [...container.querySelectorAll<HTMLElement>('.dock .list-row[data-state="live"]')].map((row) => row.dataset["thread"]);
		expect(live?.textContent).toBe("Live3");
		expect(container.querySelector("button.composer-threads")?.getAttribute("aria-label")).toBe("3 threads running");
		const key = container.querySelector<HTMLButtonElement>("button.composer-threads")!;
		await act(async () => void key.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		const rows = [...container.querySelectorAll<HTMLElement>(".composer-threads-panel .list-row[data-thread]")].map((row) => row.dataset["thread"]);
		expect(rows).toEqual(listed);
	});

	test("the band has no side-thread chip; a work thread at rest is reached from All threads at the panel's foot, in the pane's list", async () => {
		// desk-threads, decisions 4 and 5 (George, 2026-11-12).
		const { App } = await import("../src/App");
		await act(async () => root!.render(<App />));
		await settle();
		await act(async () => container.querySelector<HTMLButtonElement>('[data-teammate-row][aria-label^="Nell"]')!.click());
		await until(() => FakeCore.listDelivered, "the teammate's threads");
		await settle();
		// Nell has two work threads live, one at rest: the band names neither.
		expect(container.querySelector('[aria-label^="Side thread"]')).toBeNull();
		expect(container.querySelector(".band")?.textContent ?? "").not.toMatch(/side threads?|Side:/);
		const key = container.querySelector<HTMLButtonElement>("button.composer-threads")!;
		await act(async () => void key.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		// The one at rest is not running, so not in the panel.
		expect(container.querySelector('.composer-threads-panel [data-thread="side:s-idle"]')).toBeNull();
		const all = container.querySelector<HTMLButtonElement>(".composer-threads-panel .composer-threads-all")!;
		await act(async () => void all.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		await until(() => container.querySelector(".dock .list-group") !== null, "the Threads list");
		// Its last word was long ago, so under Active it waits in the fold of what is earlier.
		const fold = container.querySelector<HTMLButtonElement>(".dock .list-fold");
		if (fold !== null) await act(async () => fold.click());
		await until(() => container.querySelector('.dock .list-row[data-thread="side:s-idle"]') !== null, "the work thread at rest in the list");
		expect(container.querySelector(".composer-threads-panel")).toBeNull();
		expect(container.querySelector('.dock [aria-label="Threads"], .dock h2')?.textContent).toContain("Threads");
		// Its row opens it, a conversation with its field.
		await act(async () => container.querySelector<HTMLButtonElement>('.dock .list-row[data-thread="side:s-idle"]')!.click());
		await settle();
		expect(container.querySelector(".dock textarea")).not.toBeNull();
	});

	test("the working slot opens the running turn's path in the pane, and pressed again closes it (epic-42)", async () => {
		// Outcomes, not work (George, 2026-11-12): the path is one press away, in the pane, never in the pill.
		const { App } = await import("../src/App");
		await act(async () => root!.render(<App />));
		await settle();
		await act(async () => container.querySelector<HTMLButtonElement>('[data-teammate-row][aria-label^="Nell"]')!.click());
		await until(() => container.querySelector("button.composer-work") !== null, "the working slot");
		const slot = () => container.querySelector<HTMLButtonElement>("button.composer-work")!;
		// Nothing said yet: the mark alone, with no Working and no count.
		expect(slot().textContent).toBe("");
		expect(slot().getAttribute("aria-expanded")).toBe("false");
		await act(async () => void slot().dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		await until(() => container.querySelector('.dock [aria-label="Nell\'s work"]') !== null, "the path in the pane");
		expect(slot().getAttribute("aria-expanded")).toBe("true");
		expect(container.querySelector(".composer-steps")).toBeNull();
		// The field stays in use under it.
		expect(container.querySelector(".composer textarea")).not.toBeNull();
		await act(async () => void slot().dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		await until(() => container.querySelector('.dock [aria-label="Nell\'s work"]') === null, "the pane closed");
		expect(slot().getAttribute("aria-expanded")).toBe("false");
	});

	test("a thread between two teammates joins the count while one answers the other, and leaves it when the answer is in", async () => {
		// George's QA, 2026-11-12: a DM between agents never showed among the running threads.
		// The list was read before the ask, and nothing the window heard made it read it again.
		const { App } = await import("../src/App");
		await act(async () => root!.render(<App />));
		await settle();
		await act(async () => container.querySelector<HTMLButtonElement>('[data-teammate-row][aria-label^="Nell"]')!.click());
		await until(() => FakeCore.listDelivered, "the teammate's threads");
		await settle();
		const key = () => container.querySelector<HTMLButtonElement>("button.composer-threads");
		expect(key()?.textContent).toBe("3 threads");
		// Nell asks Lou; Lou answers. The core's word is her roster row, sent again.
		pairAnswering = true;
		await act(async () => FakeCore.pushRoster());
		await until(() => key()?.textContent === "4 threads", "the pair in the count");
		await act(async () => void key()!.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		const pair = container.querySelector<HTMLButtonElement>('.composer-threads-panel .list-row[data-thread="pair:lou~nell"]');
		expect(pair?.querySelector(".list-line")?.textContent).toBe("Pair · Working · Is the nightly deploy paused?");
		// The answer is in: the row goes again, and the pair with it.
		pairAnswering = false;
		await act(async () => FakeCore.pushRoster());
		await until(() => key()?.textContent === "3 threads", "the pair out of the count");
	});
});
