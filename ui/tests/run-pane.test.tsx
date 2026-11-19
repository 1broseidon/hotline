import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Persona, RosterEntry, RunningSubagent, SessionInfo, ThreadSummary, TranscriptEvent } from "../src/generated/contract";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";
import { CAPACITY_FAILURE, quietFailure } from "./failure-notice";

// task-393: a subagent's pane shows its task, its steps and words live, and its result.

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Dock } = await import("../src/components/Dock");
const { replaceDesks, wireFor } = await import("../src/desks");
const { runStream, runStateOf, stepCount } = await import("../src/components/thread/RunPane");
const { stopwatch } = await import("../src/dock");
const { toBlocks } = await import("../src/transcriptFolds");

const pork: Persona = {
	id: "pork", name: "Pork Chop", goal: "", backendId: "hotline", cwd: "/work/pork",
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
};
const session: SessionInfo = {
	personaId: "pork", state: "idle", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
	capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
};

/** Today at 1:57 PM: a run started today says only its time. */
const STARTED = new Date(new Date().setHours(13, 57, 0, 0)).getTime();
/** Each showing is a run of its own: the window keeps a thread's lines between showings. */
let key = "r0";
let serial = 0;
const TASK = "Read Dock.tsx, Conversation.tsx and LinkRow.tsx. List every place a subagent's run is shown on the desk and what each shows today. Report back as a table.";

const link = (state: "live" | "closed", end?: "done" | "cancelled" | "failed", elapsedMs?: number): TranscriptEvent => ({
	kind: "link", id: `link:run:${key}`, ts: STARTED, thread: key, threadKind: "run", personaId: "pork", title: "Audit side-pane thread rows", state,
	...(end !== undefined ? { end } : {}), ...(elapsedMs !== undefined ? { elapsedMs } : {}),
});
const task: TranscriptEvent = { kind: "user", id: "u1", ts: STARTED, text: TASK };
const read = (id: string, call: string, title: string, output?: string, ts = STARTED + 1000): TranscriptEvent => ({
	kind: "tool", id, ts, toolCallId: call, title, toolKind: "read", status: "completed",
	...(output !== undefined ? { output: [{ type: "text" as const, text: output }] } : {}),
});
const said = (id: string, text: string, ts = STARTED + 2000): TranscriptEvent => ({ kind: "agent", id, ts, text });

const running = (): RunningSubagent => ({ runId: key, title: "Audit side-pane thread rows", startedAt: STARTED, lastStep: "Read Dock.tsx", steps: 1, model: "Sonnet 5.5", stoppable: true });

type Handlers = { snapshot(items: unknown[]): void; event(item: unknown): void };
let first: TranscriptEvent[];
let runHandlers: Handlers | undefined;
let listed: ThreadSummary[];
let sent: { cmd: string; params: unknown }[];
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;
let measured: { scrollHeight: number; clientHeight: number };

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

beforeEach(async () => {
	restores = [];
	measured = { scrollHeight: 0, clientHeight: 0 };
	const installed = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	Object.assign(globalThis, { HTMLElement: installed.dom.HTMLElement });
	restores.push(() => Reflect.deleteProperty(globalThis, "HTMLElement"));
	const proto = installed.dom.HTMLElement.prototype;
	Object.defineProperty(proto, "scrollHeight", { configurable: true, get: () => measured.scrollHeight });
	Object.defineProperty(proto, "clientHeight", { configurable: true, get: () => measured.clientHeight });
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	runHandlers = undefined;
	listed = [];
	sent = [];
	const connection = wireFor("local");
	const open = spyOn(connection, "onConnection").mockImplementation((watcher) => {
		watcher("open");
		return () => {};
	});
	restores.push(() => open.mockRestore());
	const subscription = spyOn(connection, "subscribe").mockImplementation(((target: { threadId?: { kind: string; key: string } }, handlers: Handlers) => {
		const run = target.threadId?.kind === "run";
		if (run) runHandlers = handlers;
		queueMicrotask(() => handlers.snapshot(run ? first : []));
		return () => {};
	}) as Wire["subscribe"]);
	restores.push(() => subscription.mockRestore());
	const command = spyOn(connection, "command").mockImplementation((async (cmd: string, params: unknown) => {
		if (cmd === "thread.list") return listed;
		if (cmd === "thread.cancel") {
			sent.push({ cmd, params });
			return null;
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

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

/** A fresh run, so its lines are the ones the test sets. */
function next() {
	serial += 1;
	key = `r${serial}`;
}

async function show(run: RunningSubagent | undefined) {
	const entry: RosterEntry = { persona: pork, waiting: false, drawing: false, session, ...(run !== undefined ? { subagents: [run] } : {}) };
	await act(async () => {
		root.render(
			<Dock
				state={{ open: { thread: { kind: "run", key }, title: "Audit side-pane thread rows" } }}
				onState={() => {}}
				onClose={() => {}}
				onOpenTeammate={() => {}}
				entry={entry}
				roster={[entry]}
				width={360}
				onWidth={() => {}}
				overlay={false}
			/>,
		);
	});
	await settle();
}

// The band says the kind; the state is the head's line and the band heading's name.
const band = () => container.querySelector(".run-pane h2")?.getAttribute("aria-label");
const stateLine = () => container.querySelector(".run-pane .spine-state")?.textContent;
const foot = () => container.querySelector(".run-pane .spine-foot")?.textContent;
const activity = () => [...container.querySelectorAll(".run-pane .spine-rule")].find((rule) => rule.querySelector(".spine-rule-title")?.textContent === "Activity");

describe("the band and the head", () => {
	test("the band says Helper alone; the state line says how it stands, its steps and how long; Stop is at the foot while live", async () => {
		next();
		first = [link("live"), task, read("t1", "c1", "Read Dock.tsx")];
		await show(running());
		expect(band()).toBe("Helper, Working");
		expect(container.querySelector(".run-pane h2")?.textContent).toBe("Helper");
		expect(container.querySelector(".run-pane .band .run-dot, .run-pane .band .beat")).toBeNull();
		expect(stateLine()).toMatch(/^Working now· 1 step[\d:]+$/);
		expect(container.querySelector('.band button[title="Stop this helper"]')).toBeNull();
		const stop = container.querySelector<HTMLButtonElement>('.spine-foot button[title="Stop this helper"]');
		expect(stop?.textContent).toBe("Stop");
		await act(async () => stop!.click());
		expect(sent).toEqual([{ cmd: "thread.cancel", params: { thread: { kind: "run", key } } }]);

		// A harness's subagent, which the desk cannot stop, has no Stop.
		await show({ ...running(), stoppable: false });
		expect(container.querySelector('button[title="Stop this helper"]')).toBeNull();

		// Ended, its state stands still, with the steps and how long it took.
		next();
		first = [link("closed", "done", 214_000), task, read("t1", "c1", "Read Dock.tsx"), read("t2", "c2", "Read LinkRow.tsx"), said("a1", "Done.")];
		await show(undefined);
		expect(band()).toBe("Helper, Done");
		expect(stateLine()).toBe("Done· 2 steps3:34");
		expect(container.querySelector('button[title="Stop this helper"]')).toBeNull();

		next();
		first = [link("closed", "cancelled", 9_000), task];
		await show(undefined);
		expect(band()).toBe("Helper, Stopped");
		expect(runStateOf(false, "failed")).toBe("Failed");
		expect(stopwatch(3_723_000)).toBe("1:02:03");
	});

	test("its facts are three rows at most: Sent by, Model, Started", async () => {
		next();
		first = [link("live"), task];
		await show(running());
		expect(container.querySelector(".run-pane .spine-title")?.textContent).toBe("Audit side-pane thread rows");
		const rows = [...container.querySelectorAll(".run-pane .spine-facts > div")].map((row) => `${row.querySelector("dt")?.textContent}: ${row.querySelector("dd")?.textContent}`);
		expect(rows).toHaveLength(3);
		expect(rows[0]).toBe("Sent by: Pork Chop");
		expect(rows[1]).toBe("Model: Sonnet 5.5");
		expect(rows[2]).toMatch(/^Started: 1:57\s?PM$/);
	});

	test("the status mark is on Activity: mint while it works, a dim dot once done, a red cross when it failed, an amber ring while a card waits on the person", async () => {
		next();
		first = [link("live"), task, read("t1", "c1", "Read Dock.tsx")];
		await show(running());
		expect(activity()?.getAttribute("data-status")).toBe("live");
		expect(container.querySelectorAll(".run-pane .spine-rule-mark")).toHaveLength(1);

		next();
		first = [link("live"), task, { kind: "human_action", id: "h1", ts: STARTED + 2000, actionId: "act-1", reason: "Plug in the key", status: "pending" } as TranscriptEvent];
		await show(running());
		expect(activity()?.getAttribute("data-status")).toBe("blocked");

		next();
		first = [link("closed", "done", 214_000), task, said("a1", "Done.")];
		await show(undefined);
		expect(activity()?.getAttribute("data-status")).toBe("stopped");

		next();
		first = [link("closed", "failed", 9_000), task];
		await show(undefined);
		expect(activity()?.getAttribute("data-status")).toBe("failed");
	});
});

describe("the task", () => {
	test("The task it was given, Asked by its teammate, at reading size on a rule, six lines until Show all, Show less", async () => {
		measured = { scrollHeight: 400, clientHeight: 132 };
		next();
		first = [link("live"), task, read("t1", "c1", "Read Dock.tsx")];
		await show(running());
		// desk-threads, decision 2: what it was asked, and by whom.
		const card = container.querySelector('.run-pane section[aria-label="Asked by Pork Chop"]');
		expect(card?.querySelector(".spine-rule-title")?.textContent).toBe("Asked by Pork Chop");
		expect(card?.querySelector(".spine-said")?.textContent).toBe(TASK);
		expect(card?.querySelector(".spine-clamp")?.hasAttribute("data-open")).toBe(false);
		const more = card?.querySelector<HTMLButtonElement>(".spine-more");
		expect(more?.textContent).toBe("Show all");
		await act(async () => more!.click());
		expect(card?.querySelector(".spine-clamp")?.hasAttribute("data-open")).toBe(true);
		expect(more?.textContent).toBe("Show less");
		// The task is the card, not a line of the stream too.
		expect(container.querySelector(".pane-stream")?.textContent).not.toContain(TASK);
	});
});

describe("the stream", () => {
	test("a failed turn in the stream says its title and plain words, the provider's report behind Details, never the raw envelope", async () => {
		next();
		first = [link("closed", "failed", 9_000), task, read("t1", "c1", "Read Dock.tsx"), { kind: "notice", id: "n1", ts: STARTED + 3000, level: "error", text: CAPACITY_FAILURE }];
		await show(undefined);
		const stream = container.querySelector<HTMLDivElement>(".pane-stream")!;
		expect(quietFailure(stream)).toEqual({
			title: "Turn failed",
			said: "The activity could not finish. Expand the details for the reported error.",
			open: false,
			shown: "Turn failedThe activity could not finish. Expand the details for the reported error.",
		});
		expect(stream.textContent).not.toContain("hotlineFailure");
		expect(stream.querySelector("details.error-line pre")?.textContent).toContain("The model is currently at capacity due to high demand.");
	});

	test("Steps and words in one stream that follows the newest until scrolled up; a Latest pill returns; a step opens to its output", async () => {
		next();
		first = [link("live"), task, read("t1", "c1", "Read Dock.tsx", "export function Dock() {}"), said("a1", "The right pane opens a run through ThreadView.")];
		await show(running());
		const stream = container.querySelector<HTMLDivElement>(".pane-stream")!;
		// In the order they came: a step, then the words.
		const parts = [...stream.querySelectorAll(".trail-step, .pane-said")].map((part) => part.className.includes("pane-said") ? "words" : "step");
		expect(parts).toEqual(["step", "words"]);
		// Its Activity: the words are the subagent's, a line of its work under its name, never a bubble.
		expect(activity()?.querySelector(".spine-rule-title")?.textContent).toBe("Activity");
		expect(stream.querySelector(".pane-said-who")?.textContent).toBe("Helper");
		expect(stream.querySelector(".speech, .said-name")).toBeNull();
		expect(container.querySelector(".run-pane textarea, .run-pane input")).toBeNull();

		// A step opens to what it did.
		const step = stream.querySelector<HTMLButtonElement>(".trail-head")!;
		expect(step.getAttribute("aria-expanded")).toBe("false");
		await act(async () => step.click());
		expect(stream.querySelector(".trail-detail")?.textContent).toContain("export function Dock() {}");

		// It follows the newest as it lands.
		measured = { scrollHeight: 800, clientHeight: 200 };
		await act(async () => runHandlers?.event(read("t2", "c2", "Read LinkRow.tsx", undefined, STARTED + 3000)));
		expect(stream.scrollTop).toBe(800);
		expect(container.querySelector(".work-newest")).toBeNull();

		// Scrolled up, it stays put and offers Latest, which goes back down.
		stream.scrollTop = 100;
		await act(async () => stream.dispatchEvent(new window.Event("scroll")));
		await act(async () => runHandlers?.event(said("a2", "RunningSubagent has no last step.", STARTED + 4000)));
		expect(stream.scrollTop).toBe(100);
		const latest = container.querySelector<HTMLButtonElement>(".work-newest");
		expect(latest?.textContent).toBe("Latest");
		await act(async () => latest!.click());
		expect(container.querySelector(".work-newest")).toBeNull();
		expect(stepCount([...first, read("t2", "c2", "x"), read("t2", "c2", "x")])).toBe(2);
	});
});

describe("the result", () => {
	test("A Result card closes the run; the foot says whom it reports to, then that it reported", async () => {
		next();
		first = [link("live"), task, read("t1", "c1", "Read Dock.tsx"), said("a1", "Looking at the rows.")];
		await show(running());
		expect(container.querySelector(".pane-result")).toBeNull();
		expect(foot()).toBe("Reports to Pork Chop when it finishes.Stop");

		next();
		first = [link("closed", "done", 214_000), task, read("t1", "c1", "Read Dock.tsx"), said("a1", "Looking at the rows."), said("a2", "A run is shown on 6 surfaces.", STARTED + 200_000), { kind: "turn", id: "t9", ts: STARTED + 214_000, stopReason: "end_turn" }];
		await show(undefined);
		const result = container.querySelector(".pane-result");
		expect(result?.querySelector(".spine-rule")?.textContent).toBe("Result3:34");
		expect(result?.querySelector(".spine-said")?.textContent).toContain("A run is shown on 6 surfaces.");
		// The result closes the stream: the last thing in it, said once.
		expect(container.querySelector(".pane-stream > :last-child")?.className).toContain("pane-result");
		expect([...container.querySelectorAll(".pane-stream .pane-said-text")].map((one) => one.textContent)).toEqual(["Looking at the rows."]);
		expect(foot()).toMatch(/^Reported to Pork Chop at 2:00\s?PM\.$/);

		// A run stopped before it reported has no result: it says it stopped.
		next();
		first = [link("closed", "cancelled", 60_000), task, said("a1", "Looking.")];
		await show(undefined);
		expect(container.querySelector(".pane-result")).toBeNull();
		expect(foot()).toMatch(/^Stopped at 1:58\s?PM\.$/);
	});

	test("a run's own link and its turn's end are not lines of its stream, and its words while live are never the result", () => {
		const blocks = toBlocks([link("live"), task, said("a1", "Working on it."), { kind: "turn", id: "t", ts: 2, stopReason: "end_turn" }], true);
		const shown = runStream(blocks, false);
		expect(shown.task).toBe(TASK);
		expect(shown.result).toBeUndefined();
		expect(shown.blocks).toHaveLength(1);
		expect(runStream(blocks, true).result).toBe("Working on it.");
		// One message laid out in bubbles is one result; the words before it stay words.
		const bubbles = toBlocks([task, said("a1", "Looking."), said("m9", "Six surfaces."), said("m9-2", "Three changes.")], true);
		const ended = runStream(bubbles, true);
		expect(ended.result).toBe("Six surfaces.\n\nThree changes.");
		expect(ended.blocks).toHaveLength(1);
	});
});
