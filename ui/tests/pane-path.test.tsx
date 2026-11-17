import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { TranscriptEvent } from "../src/generated/contract";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

// A turn's path in the right-hand pane, on the pane's skeleton (ui/design.md,
// "The right-hand pane"): what was asked over its state line, its latest eight
// lines with the earlier ones one row that opens in place, and the page's one
// status mark on Steps.

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { WorkPage } = await import("../src/components/Work");
const { replaceDesks, wireFor } = await import("../src/desks");

const ASKED = "Can you check why the Windows job takes 40 minutes now?";
const START = new Date(new Date().setHours(11, 30, 0, 0)).getTime();

/** A step of its own kind, so each is a line of its own on the path. */
const step = (n: number, status: "completed" | "failed" | "in_progress" = "completed"): TranscriptEvent => ({
	kind: "tool",
	id: `t${n}`,
	ts: START + n * 1000,
	...(status === "in_progress" ? {} : { endedTs: START + n * 1000 + 500 }),
	toolCallId: `c${n}`,
	title: `step ${n}`,
	toolKind: "other",
	status,
	said: { key: `k${n}`, one: `Did thing ${n}`, many: `Did thing ${n} ({n})`, doing: `Doing thing ${n}` },
});
const asked: TranscriptEvent = { kind: "user", id: "u1", ts: START, text: ASKED };
const ended: TranscriptEvent = { kind: "turn", id: "e1", ts: START + 60_000, stopReason: "end_turn" };
const steps = (count: number) => Array.from({ length: count }, (_, at) => step(at + 1));

let tape: TranscriptEvent[];
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;
let serial = 0;

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

beforeEach(async () => {
	restores = [];
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
	const subscription = spyOn(connection, "subscribe").mockImplementation(((_target: unknown, handlers: { snapshot(items: unknown[]): void }) => {
		queueMicrotask(() => handlers.snapshot(tape));
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

/** The path of a teammate of its own each time: the window keeps a tape between showings. */
async function show(events: TranscriptEvent[], live = false) {
	serial += 1;
	tape = events;
	await act(async () => root.render(<WorkPage personaId={`ada${serial}`} blockId={null} name="Ada" live={live} resting={false} people={undefined} onOpenThread={() => {}} onOpenItem={() => {}} onClose={() => {}} />));
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

const lines = () => [...container.querySelectorAll("section[aria-label='Steps'] > ol.trail > .trail-step:not(.trail-fold)")].map((one) => one.querySelector(".trail-said")?.textContent);
const fold = () => container.querySelector<HTMLButtonElement>(".trail-fold-head");
const stepsRule = () => container.querySelector(".spine-rule.steps-heading");

describe("a path's head", () => {
	test("its title is what the person asked, then one state line: its steps and how long", async () => {
		await show([asked, ...steps(3), ended]);
		expect(container.querySelector(".band h2")?.textContent).toBe("Done");
		expect(container.querySelector(".work-spine-head > .spine-title")?.textContent).toBe(ASKED);
		expect(container.querySelector(".work-spine-head .spine-state-word")?.textContent).toBe("3 steps");
		expect(container.querySelector(".work-spine-head .spine-time")?.textContent).toMatch(/s$/);
	});

	test("a quiet turn nobody asked for has no title", async () => {
		await show([...steps(2), { ...ended, quiet: true }]);
		expect(container.querySelector(".work-spine-head")).not.toBeNull();
		expect(container.querySelector(".work-spine-head > .spine-title")).toBeNull();
	});
});

describe("a long path", () => {
	test("folds after eight lines: its latest eight, and one row for the earlier steps that opens in place", async () => {
		await show([asked, ...steps(12), ended]);
		expect(stepsRule()?.querySelector(".spine-rule-count")?.textContent).toBe("12");
		expect(lines()).toEqual(["Did thing 5", "Did thing 6", "Did thing 7", "Did thing 8", "Did thing 9", "Did thing 10", "Did thing 11", "Did thing 12"]);
		expect(fold()?.textContent).toBe("4 earlier stepsShow");
		// The fold row comes first, where the earlier lines were.
		expect(container.querySelector("section[aria-label='Steps'] > ol.trail")?.firstElementChild?.classList.contains("trail-fold")).toBe(true);
		await act(async () => fold()!.click());
		expect(lines()).toHaveLength(12);
		expect(lines()[0]).toBe("Did thing 1");
		expect(fold()).toBeNull();
	});

	test("eight lines or fewer never fold", async () => {
		await show([asked, ...steps(8), ended]);
		expect(lines()).toHaveLength(8);
		expect(fold()).toBeNull();
	});
});

describe("the path's status mark, on Steps", () => {
	test("a dim dot once the turn is done, a red cross when a step failed", async () => {
		await show([asked, ...steps(2), ended]);
		expect(stepsRule()?.getAttribute("data-status")).toBe("stopped");
		await show([asked, step(1), step(2, "failed"), ended]);
		expect(stepsRule()?.getAttribute("data-status")).toBe("failed");
	});

	test("mint while it works, on the heading and on the newest live line only", async () => {
		await show([asked, step(1), step(2, "in_progress"), step(3), step(4, "in_progress")], true);
		expect(stepsRule()?.getAttribute("data-status")).toBe("live");
		const all = [...container.querySelectorAll("section[aria-label='Steps'] .trail-step")];
		expect(all.map((one) => one.hasAttribute("data-now"))).toEqual([false, false, false, true]);
		expect(all.at(-1)?.getAttribute("data-status")).toBe("in_progress");
		expect([...container.querySelectorAll(".trail-dot.beat")]).toHaveLength(1);
		expect(all.at(-1)?.querySelector(".trail-dot")?.classList.contains("beat")).toBe(true);
		expect(container.querySelectorAll(".spine-rule-mark")).toHaveLength(1);
	});

	test("an amber ring while a card in the turn waits on the person", async () => {
		const card: TranscriptEvent = { kind: "permission", id: "p1", ts: START + 3000, requestId: "r1", title: "Run the installer?", options: [] };
		await show([asked, step(1), card], true);
		expect(stepsRule()?.getAttribute("data-status")).toBe("blocked");
		expect(stepsRule()?.querySelector(".spine-mark")?.hasAttribute("data-blocked")).toBe(true);
	});
});
