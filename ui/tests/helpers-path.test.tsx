import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { RosterEntry, RunningSubagent, ThreadSummary, TranscriptEvent } from "../src/generated/contract";
import type { RunLink, Step } from "../src/scheduledRuns";
import { installDom } from "./dom";

// task-448 (George, 2026-11-12, "outcomes, not work"): a helper (a subagent)
// and a command have no row in the conversation, running or finished. While
// they run, the composer's threads chip lists them; once the turn is over,
// its path in the pane does, where the turn started them, the helpers
// started together as one line ("Started 4 helpers"), each opening its own
// activity with Back to the path. A failure still reaches the line under the
// reply. The person reads "Helper", never "Subagent".

const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });
const real = await import("../src/voice/transcription");
mock.module("../src/voice/transcription", () => ({ ...real, hearsOnThisMac: async () => true }));

const user = (id: string, ts: number, text = "Profile the four peers"): TranscriptEvent => ({ kind: "user", id, ts, text });
const said = (id: string, ts: number, text: string): TranscriptEvent => ({ kind: "agent", id, ts, text });
const end = (id: string, ts: number): TranscriptEvent => ({ kind: "turn", id, ts, stopReason: "end_turn" });
const thought = (id: string, ts: number): TranscriptEvent => ({ kind: "thought", id, ts, text: "Splitting the work." });
const tool = (id: string, ts: number, key: string, one: string, status: "completed" | "failed" | "in_progress" = "completed"): TranscriptEvent => ({
	kind: "tool", id, ts, endedTs: ts + 200, toolCallId: id, title: `step ${id}`, toolKind: "other", status, said: { key, one, many: one, doing: one },
});
const run = (id: string, ts: number, title: string, more: Partial<RunLink> = {}): RunLink => ({
	kind: "link", id, ts, thread: `r-${id}`, threadKind: "run", personaId: "hana", title, state: "closed", end: "done", ...more,
});
/** One still working: no end yet. */
const running = (id: string, ts: number, title: string, more: Partial<RunLink> = {}): RunLink => ({
	kind: "link", id, ts, thread: `r-${id}`, threadKind: "run", personaId: "hana", title, state: "live", ...more,
});
const side: TranscriptEvent = { kind: "link", id: "l-side", ts: 40, thread: "s1", threadKind: "side", personaId: "hana", title: "Load test Qdrant", state: "live" };
const pair: TranscriptEvent = { kind: "link", id: "l-pair", ts: 41, thread: "hana~lou", threadKind: "pair", personaId: "hana", title: "Lou", state: "closed" };
const call: TranscriptEvent = { kind: "link", id: "l-call", ts: 42, thread: "c1", threadKind: "call", personaId: "hana", title: "Weekly recap", state: "parked" };
const job: TranscriptEvent = { kind: "agent", id: "a-job", ts: 43, text: "Nothing new overnight.", from: { kind: "job", thread: "j1", title: "Morning digest" } };

/** Two turns: one starts three helpers together and a command, one starts one helper. */
const firstTurn: TranscriptEvent[] = [
	user("u1", 1),
	{ kind: "activity", id: "d1", ts: 2, type: "thinking", message: "Profiling the four peers" },
	tool("t1", 3, "subagent", "Started a subagent"),
	run("h1", 3, "Profile group A", { elapsedMs: 66_000 }),
	thought("th1", 4),
	tool("t2", 4, "subagent", "Started a subagent"),
	run("h2", 4, "Profile group B", { end: "failed", elapsedMs: 4_000 }),
	run("h3", 5, "Profile group C", { elapsedMs: 20_000 }),
	tool("t3", 6, "read", "Read README.md"),
	run("c1", 7, "make precheck", { runKind: "shell", elapsedMs: 95_000 }),
	said("a1", 10, "All four profiled; group B needs a second look."),
	end("e1", 11),
];
const secondTurn: TranscriptEvent[] = [user("u2", 20, "Check the licenses"), tool("t4", 21, "subagent", "Started a subagent"), run("h4", 21, "Check the licenses", { elapsedMs: 30_000 }), said("a2", 30, "All MIT."), end("e2", 31)];

let act: typeof import("react").act;
let root: Root | null = null;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

describe("the conversation", () => {
	beforeEach(async () => {
		const installed = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
		teardown = installed.teardown;
		Object.assign(globalThis, { CSS: installed.dom.CSS });
		act = (await import("react")).act;
		const { createRoot } = await import("react-dom/client");
		container = document.createElement("div");
		document.body.append(container);
		root = createRoot(container);
	});
	afterEach(async () => {
		await act(async () => root?.unmount());
		await teardown();
	});

	const draw = async (events: TranscriptEvent[], more: { live?: boolean; focus?: { eventId: string; at: number } } = {}) => {
		const { Transcript } = await import("../src/components/Transcript");
		await act(async () =>
			root!.render(<Transcript personaId="hana" name="Hana" events={events} streaming={[]} live={more.live ?? false} focus={more.focus ?? null} onOpenThread={() => {}} onOpenWork={() => {}} />),
		);
	};

	test("draws no row for a helper or a command, running or finished, in a turn or started by another thread, and still draws side, pair, call and job rows", async () => {
		const going = [user("u1", 1), tool("t1", 2, "subagent", "Starting a subagent", "in_progress"), running("h1", 2, "Profile group A"), running("c1", 3, "make precheck", { runKind: "shell" })];
		await draw(going, { live: true });
		expect(container.querySelector(".link-row")).toBeNull();
		expect(container.textContent).not.toContain("Profile group A");
		expect(container.textContent).not.toContain("make precheck");

		// Finished, and one a work thread started while the conversation was between turns.
		const orphan = run("h9", 35, "Started from a work thread");
		await draw([...firstTurn, ...secondTurn, orphan, side, pair, call, job]);
		for (const gone of ["Profile group A", "make precheck", "Check the licenses · ", "Started from a work thread", "Done in"]) expect(container.textContent).not.toContain(gone);
		const rows = [...container.querySelectorAll(".link-row")].map((row) => row.getAttribute("data-kind"));
		expect(rows).toEqual(["thread", "pair", "call", "job"]);

		// A line a helper put here says where it came from in the person's word.
		await draw([user("u1", 1), { kind: "agent", id: "a-run", ts: 2, text: "Three are archived.", from: { kind: "run", thread: "r-h1" } }]);
		expect(container.querySelector(".said-name")?.textContent).toBe("From a helper");
	});

	test("a failed helper or command is marked failed on the line under the reply, first", async () => {
		const failedCommand = run("c2", 8, "npm test", { runKind: "shell", end: "failed", elapsedMs: 2_000 });
		await draw([...firstTurn.slice(0, -2), failedCommand, ...firstTurn.slice(-2)]);
		const marks = [...container.querySelectorAll(".steps-caption .outcome")].map((one) => [one.getAttribute("data-mark"), one.textContent]);
		expect(marks).toEqual([
			["failed", "Helper failed: Profile group B"],
			["failed", "Command failed: npm test"],
		]);
		const { failedWords } = await import("../src/components/transcript/Steps");
		const failed = (id: string) => run(id, 1, `Group ${id}`, { end: "failed" });
		expect(failedWords([failed("x"), failed("y")])).toEqual(["2 helpers failed"]);
		expect(failedWords([])).toEqual([]);
	});

	test("a search hit on a helper's line lands on the line under its turn's reply", async () => {
		await draw([...firstTurn, ...secondTurn], { focus: { eventId: "h2", at: 1 } });
		const lit = container.querySelector(".row-lit");
		expect(lit?.getAttribute("data-event-id")).toBe("d1");
		expect(lit?.querySelector(".steps-caption")).not.toBeNull();
	});
});

describe("the path's lines for helpers and commands", () => {
	test("helpers started together are one entry, a step between starts another, a command is its own, and the step that started a helper is not said again", async () => {
		const { pathGroups } = await import("../src/outcomes");
		const shape = (items: TranscriptEvent[]) =>
			pathGroups(items as Step[], false).map((group) => [
				group.heading?.message ?? null,
				group.entries.map((entry) =>
					entry.kind === "helpers" ? `helpers:${entry.links.map((link) => link.title).join(",")}` : entry.kind === "command" ? `command:${entry.link.title}` : entry.kind === "line" ? entry.line.text : entry.kind,
				),
			]);
		expect(shape(firstTurn.filter((event) => event.kind !== "user" && event.kind !== "agent" && event.kind !== "turn"))).toEqual([
			["Profiling the four peers", ["helpers:Profile group A,Profile group B,Profile group C", "Read README.md", "command:make precheck"]],
		]);
		// A step between, or a fresh heading, starts a new line; a failed start, which started nothing, stays.
		expect(
			shape([
				tool("t1", 1, "subagent", "Started a subagent"),
				run("h1", 1, "One"),
				tool("t2", 2, "read", "Read a file"),
				run("h2", 3, "Two"),
				{ kind: "activity", id: "d", ts: 4, type: "thinking", message: "Next" },
				run("h3", 5, "Three"),
				tool("t3", 6, "subagent", "Started a subagent", "failed"),
			]),
		).toEqual([
			[null, ["helpers:One", "Read a file", "helpers:Two"]],
			["Next", ["helpers:Three", "Started a subagent"]],
		]);
	});
});

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

describe("the path in the pane", () => {
	/** Hana's session: the turn open only for its helpers when `waiting`. */
	let waiting = false;
	let tape: TranscriptEvent[] = [];
	/** Whose conversation it is: each test has its own, since the window keeps a tape it has read for the process. */
	let who = { id: "hana", name: "Hana" };
	const persona = () => ({ ...who, goal: "", backendId: "hotline", cwd: "/work/hana", mcpPolicy: { mode: "none" as const, serverIds: [] }, skillPolicy: { mode: "none" as const, names: [] }, backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 0, updatedAt: 0 });
	const live: RunningSubagent = { runId: "r-h5", title: "Profile group D", startedAt: Date.now() - 5_000 };
	const hana = (): RosterEntry => ({
		persona: persona(),
		session: {
			personaId: who.id, state: waiting ? "thinking" : "ready", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
			capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
			...(waiting ? { awaitingSubagents: true } : {}),
		},
		waiting: false, drawing: false,
		...(waiting ? { subagents: [live] } : {}),
	});
	const listed = (): ThreadSummary[] =>
		tape.flatMap((event) => (event.kind === "link" && event.threadKind === "run" ? [{ thread: { kind: "run" as const, key: event.thread }, personaId: who.id, title: event.title, state: event.state, startedAt: event.ts, updatedAt: event.ts + 1, working: false, waiting: false, ...(event.end !== undefined ? { end: event.end } : {}), ...(event.runKind !== undefined ? { runKind: event.runKind } : {}) }] : []));
	/** A helper's own stream: its link at its head, the task it was handed, its report. */
	const streamOf = (key: string): TranscriptEvent[] => {
		const link = tape.find((event) => event.kind === "link" && event.thread === key);
		return link === undefined ? [] : [link, user(`${key}-task`, link.ts, "Profile these repos."), said(`${key}-report`, link.ts + 1, "Three of them are archived.")];
	};

	class FakeCore {
		static readonly OPEN = 1;
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
				const target = frame["sub"] as { threadId?: { kind: string; key: string } } | string;
				const thread = typeof target === "object" ? target.threadId : undefined;
				reply({ id, ok: true });
				const snapshot = target === "room" || JSON.stringify(target).includes('"roster"') ? [hana()] : thread?.kind === "dm" && thread.key === who.id ? tape : thread?.kind === "run" ? streamOf(thread.key) : [];
				reply({ sub: id, snapshot });
				return;
			}
			if (frame["cmd"] === "client.hello") reply({ id, ok: true, result: { capabilities: ["threads", "threads2"] } });
			else if (frame["cmd"] === "thread.list") reply({ id, ok: true, result: listed() });
			else if (frame["cmd"] === "aside.list") reply({ id, ok: true, result: [] });
			else reply({ id, ok: false, error: "Not in this test." });
		}
		close() {
			if (this.readyState === 3) return;
			this.readyState = 3;
			this.onclose?.();
		}
	}

	beforeEach(async () => {
		waiting = false;
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

	async function until(drawn: () => boolean, what: string) {
		for (let turn = 0; turn < 400 && !drawn(); turn++) await act(async () => void (await new Promise((resolve) => setTimeout(resolve, 5))));
		if (!drawn()) throw new Error(`Never drawn: ${what}`);
	}
	const press = async (button: Element | null | undefined) => {
		if (!(button instanceof window.HTMLElement)) throw new Error("Nothing to press");
		await act(async () => void button.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
	};
	const pathLines = () => [...container.querySelectorAll(".dock .work-page .trail-step > .trail-head")].map((head) => [head.querySelector(".trail-said")?.textContent, head.querySelector(".trail-fact")?.textContent]);
	async function openHana() {
		const { App } = await import("../src/App");
		await act(async () => root!.render(<App />));
		const row = `[data-teammate-row][aria-label^="${who.name}"]`;
		await until(() => container.querySelector(row) !== null, "the teammate in the rail");
		await press(container.querySelector(row));
		await until(() => container.querySelectorAll(".steps-caption").length > 0 || container.querySelector(".composer-work, .ambient-mark") !== null, "the conversation");
	}

	test("the line under a reply opens the path, which lists the helpers as one line that unfolds, each opening its activity with Back to the path; a lone helper opens from its line", async () => {
		tape = [...firstTurn, ...secondTurn];
		await openHana();
		expect(container.querySelector(".link-row")).toBeNull();
		await press(container.querySelectorAll(".steps-caption")[0]);
		await until(() => container.querySelector(".dock .work-page") !== null, "the path");
		expect(pathLines()).toEqual([
			["Started 3 helpers", "1 failed"],
			["Read README.md", expect.any(String)],
			["Started a command: make precheck", "Done in 1 min 35 s"],
		]);
		// The word the person reads is "helper"; the step that started one is not said again.
		expect(container.querySelector(".dock")?.textContent?.toLowerCase()).not.toContain("subagent");

		// Unfolded: each helper, how it ended and how long it took.
		const helpers = [...container.querySelectorAll(".dock .work-page .trail-step > .trail-head")].find((head) => head.textContent?.startsWith("Started 3 helpers"));
		await press(helpers);
		expect(helpers?.getAttribute("aria-expanded")).toBe("true");
		const rows = () => [...container.querySelectorAll<HTMLButtonElement>(".dock .work-page .trail-detail .trail-row")];
		expect(rows().map((row) => [row.querySelector(".trail-page")?.textContent, row.querySelector(".trail-meta")?.textContent])).toEqual([
			["Profile group A", "Done in 1 min 6 s"],
			["Profile group B", "Failed after 4 s"],
			["Profile group C", "Done in 20 s"],
		]);

		// One opens its own activity in the pane, the same page the chip's row opens.
		await press(rows()[1]);
		await until(() => container.querySelector(".dock .run-pane") !== null, "the helper's activity");
		expect(container.querySelector(".dock .run-pane")?.getAttribute("aria-label")).toBe("Helper of Hana: Profile group B");
		// Its words are under the person's word for it too.
		await until(() => container.querySelector(".dock .run-pane .pane-said-who") !== null, "the helper's words");
		expect(container.querySelector(".dock .run-pane .pane-said-who")?.textContent).toBe("Helper");

		// Back is the path, on the same turn.
		await press(container.querySelector('.dock .run-pane button[aria-label="Back"]'));
		await until(() => container.querySelector(".dock .work-page") !== null, "the path again");
		expect(pathLines()[0]).toEqual(["Started 3 helpers", "1 failed"]);

		// The next turn's lone helper is one line that opens it at once.
		await press(container.querySelector(".dock .work-page button[aria-label='Later turn']"));
		expect(pathLines()).toEqual([["Started a helper: Check the licenses", "Done in 30 s"]]);
		await press(container.querySelector(".dock .work-page .trail-step > .trail-head"));
		await until(() => container.querySelector(".dock .run-pane") !== null, "the lone helper's activity");
		expect(container.querySelector(".dock .run-pane")?.getAttribute("aria-label")).toBe("Helper of Hana: Check the licenses");
	});

	test("a turn open only for its helpers: the mark opens its path, which says Waiting and lists the one still running", async () => {
		waiting = true;
		who = { id: "iris", name: "Iris" };
		tape = [
			user("u1", 1),
			tool("t1", 2, "subagent", "Started a subagent"),
			run("h1", 2, "Profile group A", { elapsedMs: 66_000 }),
			running("h5", 3, "Profile group D"),
			said("a1", 4, "Two helpers are out."),
		];
		await openHana();
		// No line under the reply while it waits: the resting mark is the line.
		expect(container.querySelector(".steps-caption")).toBeNull();
		await until(() => container.querySelector("button.composer-work") !== null, "the resting mark");
		await press(container.querySelector("button.composer-work"));
		await until(() => container.querySelector(".dock .work-page") !== null, "the path");
		expect(container.querySelector(".dock .work-page h2")?.textContent).toBe("Waiting");
		expect(pathLines()).toEqual([["Started 2 helpers", "1 running"]]);
		// The chip lists the one still running and opens it.
		const key = container.querySelector<HTMLButtonElement>("button.composer-threads");
		expect(key?.textContent).toBe("1 thread");
		await press(key);
		const row = container.querySelector(".composer-threads-panel .list-row[data-thread='run:r-h5']");
		expect(row?.querySelector(".list-line")?.textContent).toBe("Helper · Starting");
		await press(row);
		await until(() => container.querySelector(".dock .run-pane") !== null, "the running helper");
		expect(container.querySelector(".dock .run-pane")?.getAttribute("aria-label")).toBe("Helper of Iris: Profile group D");
	});
});
