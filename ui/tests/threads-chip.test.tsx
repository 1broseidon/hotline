import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { RosterEntry, RunningSide, RunningSubagent, ThreadSummary, TranscriptEvent } from "../src/generated/contract";
import type { LiveThread } from "../src/liveThreads";
import { installDom } from "./dom";

// task-437: the desk composer counts the threads live in the session, as the
// phone's does ("2 threads ›"), lists them in a panel that grows the pill up,
// and the per-subagent chips that stood on the pill's corner are gone.

const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });
const real = await import("../src/voice/transcription");
mock.module("../src/voice/transcription", () => ({ ...real, hearsOnThisMac: async () => true }));
const { liveThreadCount, liveThreads, threadsLabel, threadsWords } = await import("../src/liveThreads");
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
const entry = (sides: RunningSide[], subagents: RunningSubagent[], state: "ready" | "thinking" = "ready"): RosterEntry => ({
	persona: persona("ada", "Ada"), session: session("ada", state), waiting: false, drawing: false,
	...(sides.length > 0 ? { sides } : {}), ...(subagents.length > 0 ? { subagents } : {}),
});

const audit: RunningSubagent = { runId: "r-audit", title: "Audit side-pane thread rows", startedAt: NOW - 134_000, lastStep: "Read Dock.tsx", steps: 4 };
const fresh: RunningSubagent = { runId: "r-fresh", title: "Check the model list", startedAt: NOW - 5_000 };
const crawl: RunningSubagent = { runId: "r-crawl", title: "Crawl 12 sites", startedAt: NOW - 40_000, kind: "shell", command: "python3 crawl.py" };
const notes: RunningSide = { sideId: "s-notes", title: "Release notes", startedAt: NOW - 580_000, working: true };
const installer: RunningSide = { sideId: "s-hand", title: "Check the installer", startedAt: NOW - 1_260_000, working: false };
const handedOver: TranscriptEvent = {
	kind: "link", id: "link:side:s-hand", ts: installer.startedAt, thread: "s-hand", threadKind: "side", personaId: "ada",
	title: installer.title, state: "live", openerId: "mack", openerName: "Mack",
};

describe("task-437 the count", () => {
	test("reads 0, 1, 2 and 9+ the phone's way, and says how many run to a screen reader", () => {
		expect(threadsWords(1)).toBe("1 thread");
		expect(threadsWords(2)).toBe("2 threads");
		expect(threadsWords(9)).toBe("9 threads");
		expect(threadsWords(12)).toBe("9+ threads");
		expect(threadsLabel(1)).toBe("1 thread running");
		expect(threadsLabel(12)).toBe("12 threads running");
		expect(liveThreadCount(entry([], []))).toBe(0);
	});

	test("desk and phone show the same count for the same session: its work threads and its runs, from the roster row", () => {
		// hotline-mobile src/now.ts `threadCount(row.sides, row.subagents)`, the number its ThreadsCount shows.
		const phone = (row: RosterEntry) => (row.sides?.length ?? 0) + (row.subagents?.length ?? 0);
		for (const row of [entry([], []), entry([], [audit]), entry([installer], [audit]), entry([notes, installer], [audit, fresh, crawl])]) {
			expect(liveThreadCount(row)).toBe(phone(row));
		}
		// A command left running in the background is a run, counted as the phone counts it; a process the session lists is not.
		const withProcess = { ...entry([], [crawl]), session: { ...session("ada"), processes: [{ id: "p1", title: "vite", startedAt: NOW, state: "running" as const }] } };
		expect(liveThreadCount(withProcess)).toBe(1);
	});

	test("says the same number as the Threads list's Live group while the threads run", () => {
		const row = entry([notes], [audit, crawl]);
		const listed: ThreadSummary[] = [
			{ thread: { kind: "side", key: notes.sideId }, personaId: "ada", title: notes.title, state: "live", startedAt: notes.startedAt, updatedAt: NOW, working: false, waiting: false },
			...[audit, crawl].map((run): ThreadSummary => ({ thread: { kind: "run", key: run.runId }, personaId: "ada", title: run.title, state: "live", startedAt: run.startedAt, updatedAt: NOW, working: false, waiting: false })),
			// What has ended is no thread of the session's, in either.
			{ thread: { kind: "run", key: "r-old" }, personaId: "ada", title: "Earlier audit", state: "closed", end: "done", startedAt: 1, updatedAt: NOW - 1_000, working: false, waiting: false },
		];
		const items = listItems({ rows: withTurns(listed, [row], "ada"), runs: row.subagents ?? [], jobs: [], asides: [], listedFor: "ada", nameOf: () => undefined, personName: "", now: NOW });
		const live = grouped(items, NOW, true).live;
		expect(live.length).toBe(3);
		expect(liveThreadCount(row)).toBe(live.length);
		expect(liveThreads(row, []).map((one) => one.id).sort()).toEqual(live.map((item) => item.id).sort());
	});

	test("counts an open work thread at rest, as the phone does, where the Threads list files it under Recent", () => {
		// The one place the two part: the roster holds the thread open, so the phone and the composer count it;
		// the list's Live group is what has a turn running.
		const row = entry([installer], []);
		const listed: ThreadSummary[] = [
			{ thread: { kind: "side", key: installer.sideId }, personaId: "ada", title: installer.title, state: "live", startedAt: installer.startedAt, updatedAt: NOW - 60_000, working: false, waiting: false },
		];
		const items = listItems({ rows: withTurns(listed, [row], "ada"), runs: [], jobs: [], asides: [], listedFor: "ada", nameOf: () => undefined, personName: "", now: NOW });
		const groups = grouped(items, NOW, true);
		expect(liveThreadCount(row)).toBe(1);
		expect(groups.live.length).toBe(0);
		expect(groups.recent.map((item) => item.id)).toEqual(["side:s-hand"]);
	});

	test("lists only the live threads, each with its kind, what it is doing and since when", () => {
		const rows = liveThreads(entry([notes, installer], [audit, fresh, crawl]), [handedOver]);
		expect(rows.map((one) => [one.title, one.what, one.doing, one.working])).toEqual([
			["Release notes", "Work thread", "Working", true],
			["Check the installer", "Handoff from Mack", "Open", false],
			["Audit side-pane thread rows", "Subagent", "Read Dock.tsx", true],
			["Check the model list", "Subagent", "Starting", true],
			["Crawl 12 sites", "Command", "python3 crawl.py", true],
		]);
		expect(rows.map((one) => one.thread)).toEqual([
			{ kind: "side", key: "s-notes" },
			{ kind: "side", key: "s-hand" },
			{ kind: "run", key: "r-audit" },
			{ kind: "run", key: "r-fresh" },
			{ kind: "run", key: "r-crawl" },
		]);
		expect(liveThreads(entry([], []), [handedOver])).toEqual([]);
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
	async function composer(rows: LiveThread[], state: "ready" | "thinking" = "ready") {
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
						threads={{ rows, isOpen: (one) => one.id === "run:r-audit", onOpen: (one) => opened.push(one) }}
					/>
				</div>,
			),
		);
	}
	const key = () => container.querySelector<HTMLButtonElement>("button.composer-threads");
	const panel = () => container.querySelector<HTMLElement>(".composer-threads-panel");
	const rows = () => [...container.querySelectorAll<HTMLButtonElement>(".composer-threads-panel .list-row")];
	const field = () => container.querySelector("textarea")!;
	const press = async (target: Element) => act(async () => void target.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
	const down = async (target: Element) => act(async () => void target.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true })));
	const keyDown = async (target: Element, key: string) => act(async () => void target.dispatchEvent(new window.KeyboardEvent("keydown", { key, bubbles: true })));
	const live = (row: RosterEntry) => liveThreads(row, [handedOver]);

	beforeEach(() => {
		opened.length = 0;
		cancels.length = 0;
	});

	test("is absent at zero, and reads 1 thread, 2 threads, 9+ threads at the bottom row's right as a button labelled how many run", async () => {
		await composer([]);
		expect(key()).toBeNull();
		// At rest with nothing live the pill stays one row.
		expect(container.querySelector(".composer")?.hasAttribute("data-work")).toBe(false);

		await composer(live(entry([], [audit])));
		expect(key()?.textContent).toBe("1 thread");
		expect(key()?.getAttribute("aria-label")).toBe("1 thread running");
		expect(container.querySelector(".composer")?.hasAttribute("data-work")).toBe(true);

		await composer(live(entry([installer], [audit])));
		expect(key()?.textContent).toBe("2 threads");
		expect(key()?.getAttribute("aria-label")).toBe("2 threads running");
		expect(key()?.title).toBe("Show threads");

		const many = Array.from({ length: 12 }, (_, at): RunningSubagent => ({ runId: `r${at}`, title: `Check ${at}`, startedAt: NOW }));
		await composer(live(entry([], many)));
		expect(key()?.textContent).toBe("9+ threads");
		expect(key()?.getAttribute("aria-label")).toBe("12 threads running");
	});

	test("is reached by Tab: a button after the field, before the keys", async () => {
		await composer(live(entry([], [audit])));
		const focusable = [...container.querySelectorAll<HTMLElement>("button, textarea")].filter((one) => one.tabIndex >= 0);
		expect(key()?.tagName).toBe("BUTTON");
		expect(focusable.indexOf(key()!)).toBe(focusable.indexOf(field()) + 1);
		expect(focusable.at(-1)?.getAttribute("aria-label")).toBe("Dictate");
	});

	test("opens a panel of only the live threads, titled how many run, each row its kind, what it is doing and how long", async () => {
		await composer(live(entry([installer], [audit])));
		expect(panel()).toBeNull();
		await press(key()!);
		expect(key()?.getAttribute("aria-expanded")).toBe("true");
		expect(panel()?.getAttribute("aria-label")).toBe("2 threads running");
		expect(panel()?.querySelector("h3")?.textContent).toBe("2 threads running");
		expect(key()?.getAttribute("aria-controls")).toBe(panel()?.id);
		// The pill's first row: inside it, before the field.
		expect(panel()?.parentElement?.className).toBe("composer");
		expect(rows().map((row) => [row.querySelector(".list-title-text")?.textContent, row.querySelector(".list-line")?.textContent])).toEqual([
			["Check the installer", "Handoff from Mack · Open"],
			["Audit side-pane thread rows", "Subagent · Read Dock.tsx"],
		]);
		const when = rows().map((row) => row.querySelector(".list-when")?.textContent ?? "");
		expect(when.every((one) => /^\d+:\d\d(:\d\d)?$/.test(one))).toBe(true);
		expect(rows()[1]?.getAttribute("aria-label")).toStartWith("Subagent: Audit side-pane thread rows, Read Dock.tsx, ");
		// The one open in the pane says so, as the chip's pressed state did.
		expect(rows()[1]?.getAttribute("aria-current")).toBe("true");
		expect(rows()[0]?.hasAttribute("aria-current")).toBe(false);
	});

	test("a row opens its thread and the panel closes", async () => {
		await composer(live(entry([installer], [audit])));
		await press(key()!);
		await press(rows()[1]!);
		expect(opened.map((one) => one.thread)).toEqual([{ kind: "run", key: "r-audit" }]);
		expect(panel()).toBeNull();
	});

	test("Escape closes it, before it reaches the turn, and focus returns to the control", async () => {
		await composer(live(entry([], [audit])), "thinking");
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
		await composer(live(entry([], [audit])));
		await press(key()!);
		await down(field());
		field().focus();
		expect(panel()).not.toBeNull();
		expect(document.activeElement).toBe(field());
		await down(container.querySelector(".away")!);
		expect(panel()).toBeNull();
	});

	test("the control pressed again closes it", async () => {
		await composer(live(entry([], [audit])));
		await press(key()!);
		expect(panel()).not.toBeNull();
		await press(key()!);
		expect(panel()).toBeNull();
		expect(key()?.getAttribute("aria-expanded")).toBe("false");
		expect(key()?.title).toBe("Show threads");
	});

	test("opened from the keyboard it takes focus to its first row; arrows walk the rows, and down past the last is the control again", async () => {
		await composer(live(entry([notes, installer], [audit])));
		key()!.focus();
		// Enter or Space on a button is a click with no pointer behind it.
		await act(async () => key()!.click());
		expect(document.activeElement).toBe(rows()[0]!);
		await keyDown(rows()[0]!, "ArrowDown");
		expect(document.activeElement).toBe(rows()[1]!);
		await keyDown(rows()[1]!, "End");
		expect(document.activeElement).toBe(rows()[2]!);
		await keyDown(rows()[2]!, "ArrowUp");
		expect(document.activeElement).toBe(rows()[1]!);
		await keyDown(rows()[1]!, "Home");
		expect(document.activeElement).toBe(rows()[0]!);
		await keyDown(rows()[0]!, "End");
		await keyDown(rows()[2]!, "ArrowDown");
		expect(document.activeElement).toBe(key());
		// From the control, up reaches the row nearest it.
		await keyDown(key()!, "ArrowUp");
		expect(document.activeElement).toBe(rows()[2]!);
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
						threads={{ rows: live(entry([], [audit])), isOpen: () => false, onOpen: () => {} }}
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
		await composer(live(entry([], [audit])));
		await press(key()!);
		await composer([]);
		expect(key()).toBeNull();
		await composer(live(entry([], [fresh])));
		expect(panel()).toBeNull();
	});
});

describe("task-437 in the conversation", () => {
	// Its own teammate: the window keeps each thread's store by desk and id for the
	// process, so an "ada" another file opened would answer with that file's socket.
	const nell = (): RosterEntry => ({ ...entry([installer], [audit, crawl], "thinking"), persona: persona("nell", "Nell"), session: session("nell", "thinking") });
	const handedToNell: TranscriptEvent = { ...handedOver, personaId: "nell" };

	class FakeCore {
		static readonly OPEN = 1;
		/** The conversation's tape, with the handoff's link, has reached the window. */
		static tapeDelivered = false;
		readyState = 1;
		onopen: (() => void) | null = null;
		onmessage: ((message: { data: string }) => void) | null = null;
		onclose: (() => void) | null = null;
		onerror: (() => void) | null = null;
		constructor(readonly url: string) {
			setTimeout(() => this.onopen?.(), 0);
		}
		send(data: string) {
			const frame = JSON.parse(data) as Record<string, unknown>;
			const id = frame["id"];
			const reply = (answer: Record<string, unknown>) => setTimeout(() => this.onmessage?.({ data: JSON.stringify(answer) }), 0);
			if ("sub" in frame) {
				const target = JSON.stringify(frame["sub"]);
				reply({ id, ok: true });
				if (target.includes('"dm"')) {
					setTimeout(() => {
						this.onmessage?.({ data: JSON.stringify({ sub: id, snapshot: [handedToNell] }) });
						FakeCore.tapeDelivered = true;
					}, 0);
				} else reply({ sub: id, snapshot: target.includes('"roster"') ? [nell()] : [] });
				return;
			}
			if (frame["cmd"] === "client.hello") reply({ id, ok: true, result: { capabilities: ["threads", "threads2"] } });
			else if (frame["cmd"] === "thread.list") reply({ id, ok: true, result: [] });
			else reply({ id, ok: false, error: "Not in this test." });
		}
		close() {
			if (this.readyState === 3) return;
			this.readyState = 3;
			this.onclose?.();
		}
	}

	beforeEach(async () => {
		FakeCore.tapeDelivered = false;
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
		// Who handed the thread over is the tape's word: its link line is drawn once the tape is in.
		// Who handed the thread over is the tape's word, and the tape is asked for once the socket is open, after the roster.
		await until(() => FakeCore.tapeDelivered, "the conversation's tape");
		await settle();
		// No chip stands on the pill's corner for the runs: nothing there at all, with no process listed.
		expect(container.querySelector(".composer-corner")).toBeNull();
		expect(container.querySelector('[aria-label^="Subagent working"], [aria-label^="Command running"], [aria-label$="subagents working"]')).toBeNull();
		const key = container.querySelector<HTMLButtonElement>("button.composer-threads");
		expect(key?.textContent).toBe("3 threads");
		await act(async () => void key!.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		const rows = [...container.querySelectorAll<HTMLButtonElement>(".composer-threads-panel .list-row")];
		expect(rows.map((row) => row.querySelector(".list-line")?.textContent)).toEqual([
			"Handoff from Mack · Open",
			"Subagent · Read Dock.tsx",
			"Command · python3 crawl.py",
		]);
		// The subagent's chip opened its run in the pane; its row does the same.
		await act(async () => void rows[1]!.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		await settle();
		expect(container.querySelector(".composer-threads-panel")).toBeNull();
		expect(container.querySelector(".dock .run-pane")?.getAttribute("aria-label")).toBe("Subagent of Nell: Audit side-pane thread rows");
		// Open in the pane, its row says so.
		await act(async () => void key!.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		const again = [...container.querySelectorAll<HTMLButtonElement>(".composer-threads-panel .list-row")];
		expect(again.map((row) => row.getAttribute("aria-current"))).toEqual([null, "true", null]);
		// The command's chip opened its run too.
		await act(async () => void again[2]!.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		await settle();
		expect(container.querySelector(".dock .run-pane, .dock [aria-label^='Command']")).not.toBeNull();
	});
});
