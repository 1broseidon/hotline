import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { PlanUsage } from "../src/generated/contract";
import { installDom } from "./dom";

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

beforeEach(async () => {
	({ teardown } = installDom());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

const hour = 3_600_000;
/** Two of five hours gone at 9%; three of seven days gone at 39%. */
const plan = (five: number, week: number): PlanUsage => ({
	at: Date.now(),
	plan: "Max",
	windows: [
		{ kind: "five_hour", usedPercent: five, status: "allowed", windowMinutes: 300, resetsAt: Date.now() + 3 * hour },
		{ kind: "seven_day", usedPercent: week, status: "allowed", windowMinutes: 10_080, resetsAt: Date.now() + 4 * 24 * hour },
	],
});

async function gauge(reading: PlanUsage | undefined, onOpen?: () => void) {
	const { PlanGauge } = await import("../src/components/PlanGauge");
	await act(async () => {
		root.render(<PlanGauge plan={reading} agent="Claude Code" mark={null} onOpen={onOpen} />);
	});
	return container.querySelector<HTMLButtonElement>("button.plan-gauge");
}

const card = () => document.querySelector<HTMLElement>('[role="tooltip"].plan-card');
const rings = (button: HTMLElement) => [...button.querySelectorAll<SVGGElement>("g[data-ring]")].map((ring) => [ring.dataset["ring"], ring.dataset["used"]]);

test("the gauge is there only while the login has a plan reading", async () => {
	expect(await gauge(undefined)).toBeNull();
	expect(await gauge({ at: 1, windows: [] })).toBeNull();
	expect(await gauge(plan(9, 39))).not.toBeNull();
});

test("weekly is the outer ring and the 5-hour the inner, each as far round as it is used; one window is one ring", async () => {
	const button = (await gauge(plan(9, 39)))!;
	expect(rings(button)).toEqual([
		["outer", "39"],
		["inner", "9"],
	]);
	expect(button.getAttribute("aria-label")).toBe("Plan usage: 5h 9%, weekly 39%");
	expect(button.dataset["near"]).toBeUndefined();
	const one = (await gauge({ at: 1, windows: [{ kind: "five_hour", usedPercent: 0, status: "allowed" }] }))!;
	expect(rings(one)).toEqual([["outer", "0"]]);
	// Nothing used has a track and no arc.
	expect(one.querySelectorAll(".plan-gauge-arc").length).toBe(0);
	expect(one.querySelectorAll(".plan-gauge-track").length).toBe(1);
});

test("the gauge turns amber when any window is 80% used or warned", async () => {
	expect((await gauge(plan(80, 39)))!.dataset["near"]).toBe("true");
	expect((await gauge(plan(79, 39)))!.dataset["near"]).toBeUndefined();
	const warned = plan(20, 30);
	warned.windows[1] = { ...warned.windows[1]!, status: "warning" };
	expect((await gauge(warned))!.dataset["near"]).toBe("true");
});

test("hovered, the card names the agent and plan, with each window's line and its runway; leaving lets it go", async () => {
	let opened = 0;
	const button = (await gauge(plan(9, 39), () => opened++))!;
	expect(card()).toBeNull();
	await act(async () => button.dispatchEvent(new window.MouseEvent("mouseover", { bubbles: true })));
	expect(opened).toBe(1);
	const shown = card()!;
	expect(button.getAttribute("aria-describedby")).toBe(shown.id);
	expect(shown.querySelector(".model-head-name")?.textContent).toBe("Claude Code");
	expect(shown.querySelector(".model-head-kind")?.textContent).toBe("Max subscription");
	const lines = [...shown.querySelectorAll<HTMLElement>(".plan-row")];
	expect(lines.map((line) => [line.querySelector(".plan-name")?.textContent, line.querySelector(".plan-use")?.textContent])).toEqual([
		["5h", "9%"],
		["Weekly", "39%"],
	]);
	// Two of five hours gone: the pace tick at 40%.
	expect(Number.parseFloat(lines[0]!.querySelector<HTMLElement>(".plan-tick")!.style.left)).toBeCloseTo(40, 0);
	const notes = [...shown.querySelectorAll(".plan-runway")].map((line) => line.textContent);
	expect(notes.length).toBe(2);
	for (const note of notes) expect(note).toMatch(/^Resets .+ · Runway: lasts past the reset$/);
	await act(async () => button.dispatchEvent(new window.MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body })));
	await act(async () => new Promise((resolve) => setTimeout(resolve, 200)));
	expect(card()).toBeNull();
});

test("focus shows the card too; a click pins it until a click elsewhere or Escape", async () => {
	const button = (await gauge(plan(9, 39)))!;
	await act(async () => button.focus());
	expect(card()).not.toBeNull();
	await act(async () => button.blur());
	expect(card()).toBeNull();

	await act(async () => button.click());
	expect(card()).not.toBeNull();
	await act(async () => button.dispatchEvent(new window.MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body })));
	await act(async () => new Promise((resolve) => setTimeout(resolve, 200)));
	expect(card()).not.toBeNull();
	await act(async () => document.body.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true })));
	expect(card()).toBeNull();

	await act(async () => button.click());
	expect(card()).not.toBeNull();
	await act(async () => button.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
	expect(card()).toBeNull();
});
