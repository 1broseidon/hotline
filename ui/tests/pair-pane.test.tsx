import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Aside, ThreadSummary, TranscriptEvent } from "../src/generated/contract";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

/*
 * The pair's pane, the one conversation between two teammates: one press opens
 * it, and an ask, a handoff with its work and an aside all read there.
 */

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { PairPane, handoffCut, handoffWork, pairLines } = await import("../src/components/thread/PairPane");
const { replaceDesks, wireFor } = await import("../src/desks");

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Handlers = { snapshot(items: unknown[]): void; event(item: unknown): void };

const poe = { id: "poe", name: "Poe" };
const mack = { id: "mack", name: "Mack" };
const key = "mack~poe";
const user = (id: string, text: string, handoffThread?: string): TranscriptEvent => ({ kind: "user", id, ts: 1, text, ...(handoffThread !== undefined ? { handoffThread } : {}) });
const agent = (id: string, text: string, handoffThread?: string): TranscriptEvent => ({ kind: "agent", id, ts: 9, text, ...(handoffThread !== undefined ? { handoffThread } : {}) });
const handedIn = (request: string, ts = 2): TranscriptEvent => ({
	kind: "delivery", id: `handoff:${request}`, ts, text: "The words",
	cause: { kind: "handoff", requestId: request, personaId: "mack", name: "Mack", threadKey: key, about: "The words" },
});
const step = (id: string, ts: number, title: string): TranscriptEvent => ({ kind: "tool", id, ts, toolCallId: id, title, status: "completed" });
const link = (over: Partial<Extract<TranscriptEvent, { kind: "link" }>>): TranscriptEvent => ({ kind: "link", id: "link:side:w", ts: 1, thread: "w", threadKind: "side", title: "Fix it", state: "live", ...over });
const aside = (id: string, by: string, to: string, text: string, ts: number): Aside => ({ id, ts, to, by, text, level: "note", from: { thread: { kind: "pair", key } } });

/** What each work thread holds when the pane subscribes to it. */
let streams: Map<string, TranscriptEvent[]>;
let handlers: Map<string, Handlers>;
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;

beforeEach(async () => {
	restores = [];
	streams = new Map();
	handlers = new Map();
	const installed = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	const connection = wireFor("local");
	const open = spyOn(connection, "onConnection").mockImplementation((watcher) => {
		watcher("open");
		return () => {};
	});
	restores.push(() => open.mockRestore());
	const subscription = spyOn(connection, "subscribe").mockImplementation(((target: { threadId?: { key: string } }, listening: Handlers) => {
		const thread = target.threadId?.key ?? "";
		handlers.set(thread, listening);
		queueMicrotask(() => listening.snapshot(streams.get(thread) ?? []));
		return () => {};
	}) as Wire["subscribe"]);
	restores.push(() => subscription.mockRestore());
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

const row: ThreadSummary = { thread: { kind: "pair", key }, personaId: poe.id, withPersonaId: mack.id, state: "live", startedAt: 1, updatedAt: 2, working: false, waiting: false };

async function show(events: TranscriptEvent[], workingSides: string[] = [], asides: Aside[] = []) {
	await act(async () => {
		root.render(<PairPane threadKey={key} self={poe} other={mack} row={row} events={events} asides={asides} workingSides={workingSides} people={new Map()} onBack={() => {}} onClose={() => {}} />);
	});
	await act(async () => {
		await new Promise((done) => setTimeout(done, 0));
	});
}

const names = () => [...container.querySelectorAll(".pair-line .said-name")].map((name) => name.textContent);
const pageText = () => container.textContent ?? "";

describe("a pair's pane", () => {
	test("an ask and its answer are plain lines under the speaker's face and name, with no chip", async () => {
		await show([user("r1", "What port?"), agent("result:r1", "8080.")]);
		expect(names()).toEqual(["Mack", "Poe"]);
		expect(container.querySelector(".pair-chip")).toBeNull();
		expect(container.querySelector(".pair-work")).toBeNull();
	});

	test("a line written before lines were stamped stays a plain line, in its own words", () => {
		const lines = pairLines(key, [user("r1", "Fix the badge.")], poe, mack);
		expect(lines).toEqual([{ kind: "said", id: "r1", who: mack, right: true, text: "Fix the badge." }]);
	});

	test("a running handoff is its request in full and its steps as they land, with nothing to press", async () => {
		const long = `Fix the badge.\n\n${"Every line of this brief counts. ".repeat(200)}`;
		streams.set("run-1", [link({ thread: "run-1" }), handedIn("r1"), step("s1", 3, "Read Dock.tsx")]);
		await show([user("r1", long, "run-1")], ["run-1"]);
		expect(container.querySelector(".pair-line .pair-chip")?.textContent).toBe("Handoff");
		expect(pageText()).toContain("Fix the badge.");
		expect(pageText().split("Every line of this brief counts.").length).toBe(201);
		expect(pageText()).not.toContain("Show all");
		expect(pageText()).not.toContain("Open ›");
		expect(pageText()).toContain("Read Dock.tsx");
		await act(async () => handlers.get("run-1")?.event(step("s2", 4, "Ran the tests")));
		expect(pageText()).toContain("Ran the tests");
	});

	test("a finished handoff reads request, steps, result in order, its words said once and no status line", async () => {
		streams.set("done-1", [link({ thread: "done-1", state: "closed", end: "agent" }), handedIn("r1"), step("s1", 3, "Read Dock.tsx"), agent("a1", "Three tests pass."), { kind: "turn", id: "t1", ts: 5, stopReason: "end_turn" }]);
		await show([user("r1", "Add mul.", "done-1"), agent("result:r1", "Three tests pass.", "done-1")]);
		const text = pageText();
		expect(text.indexOf("Add mul.")).toBeLessThan(text.indexOf("Read Dock.tsx"));
		expect(text.indexOf("Read Dock.tsx")).toBeLessThan(text.indexOf("Three tests pass."));
		expect(text.split("Three tests pass.").length).toBe(2);
		expect([...container.querySelectorAll(".pair-chip")].map((chip) => chip.textContent)).toEqual(["Handoff", "Handoff"]);
		expect(names()).toEqual(["MackHandoff", "PoeHandoff"]);
		expect(container.querySelector(".pair-work-cut")).toBeNull();
		expect(text).not.toContain("Done");
	});

	test("a failed or stopped handoff says so in one plain line, and only the latest into a thread", async () => {
		expect(handoffCut([link({ state: "closed", end: "failed" })])).toBe("Handoff failed");
		expect(handoffCut([link({ state: "closed", end: "stopped" })])).toBe("Handoff stopped");
		expect(handoffCut([link({ handoffEnded: true })])).toBe("Handoff stopped");
		expect(handoffCut([link({ state: "closed", end: "agent" })])).toBeNull();
		expect(handoffCut([link({})])).toBeNull();
		streams.set("cut-1", [link({ thread: "cut-1", state: "closed", end: "stopped" }), handedIn("r1"), handedIn("r2", 6)]);
		await show([user("r1", "First.", "cut-1"), user("r2", "Second.", "cut-1")]);
		expect([...container.querySelectorAll(".pair-work-cut")].map((line) => line.textContent)).toEqual(["Handoff stopped"]);
	});

	test("each handoff into a thread reads only its own work", () => {
		const stream = [link({}), handedIn("r1"), step("s1", 3, "First step"), handedIn("r2", 4), step("s2", 5, "Second step")];
		expect(handoffWork(stream, "r1", undefined).map((event) => event.id)).toEqual(["s1"]);
		expect(handoffWork(stream, "r2", undefined).map((event) => event.id)).toEqual(["s2"]);
		expect(handoffWork(stream, "unseen", undefined)).toEqual([]);
		const lines = pairLines(key, [user("r1", "First.", "w"), user("r2", "Second.", "w")], poe, mack);
		expect(lines.map((line) => (line.kind === "said" ? line.work?.latest : undefined))).toEqual([false, true]);
	});

	test("an aside between the two is a line with its chip at its time; the ones repeating a result or from elsewhere are not", async () => {
		const asides = [
			aside("a1", "poe", "mack", "The Q3 numbers are in /shared/q3.csv", 5),
			aside("a2", "ada", "poe", "From someone else", 6),
			aside("result:r1", "mack", "poe", "The result again", 7),
			aside("stopped:r1", "mack", "poe", "Stopped again", 7),
			{ ...aside("a3", "poe", "mack", "Quill looped you in.", 7), loopedIn: true },
		];
		await show([user("r1", "First."), agent("result:r1", "Second.")], [], asides);
		expect(names()).toEqual(["Mack", "PoeAside", "Poe"]);
		expect(pageText()).toContain("The Q3 numbers are in /shared/q3.csv");
		expect(pageText()).not.toContain("From someone else");
		expect(pageText()).not.toContain("again");
		expect(pageText()).not.toContain("looped you in");
	});
});
