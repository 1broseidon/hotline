import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { ComputerStatus } from "../src/generated/contract";
import { installDom } from "./dom";

/*
 * The conversation's computer key asks the desk whether the teammate's
 * computer runs, since the desk says nothing on the wire when it starts or
 * stops. It used to ask every 5 s for as long as the conversation was open,
 * window in front or not: a wire round trip that woke an idle page every
 * 5 s, and in WebKit every such wake leaves 512 KB regions in the web
 * process that it never gives back. Now it asks every 10 s in front, every
 * minute behind another app, never while it cannot be seen, and at once
 * when something says the answer may have moved.
 */

let teardown: () => Promise<void>;
let restores: (() => void)[];
let asked: number;
let status: ComputerStatus;
let focused: boolean;
let visibility: "visible" | "hidden";
/** The key's own timers, by delay; the rest (React's) run as they would. */
let timers: Map<number, { fn: () => void; ms: number }>;
let intervals: number[];
let act: typeof import("react").act;
let root: Root;

beforeEach(async () => {
	({ teardown } = installDom());
	restores = [];
	asked = 0;
	focused = false;
	visibility = "visible";
	timers = new Map();
	intervals = [];
	const { wire } = await import("../src/wire");
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string) => {
		if (cmd !== "computer.status") throw new Error(`Not in this test: ${cmd}`);
		asked++;
		return { ...status };
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
	const hasFocus = spyOn(document, "hasFocus").mockImplementation(() => focused);
	restores.push(() => hasFocus.mockRestore());
	Object.defineProperty(document, "visibilityState", { configurable: true, get: () => visibility });
	const { COMPUTER_KEY_FOCUSED_MS, COMPUTER_KEY_BEHIND_MS } = await import("../src/computer");
	const ours = new Set([COMPUTER_KEY_FOCUSED_MS, COMPUTER_KEY_BEHIND_MS, 5000]);
	const realTimeout = globalThis.setTimeout;
	const realClear = globalThis.clearTimeout;
	let next = 1_000_000;
	const later = spyOn(globalThis, "setTimeout").mockImplementation(((fn: () => void, ms?: number, ...rest: unknown[]) => {
		if (ms === undefined || !ours.has(ms)) return realTimeout(fn, ms, ...rest);
		const id = next++;
		timers.set(id, { fn, ms });
		return id;
	}) as unknown as typeof setTimeout);
	const cancel = spyOn(globalThis, "clearTimeout").mockImplementation(((id: number) => {
		if (!timers.delete(id)) realClear(id);
	}) as unknown as typeof clearTimeout);
	const every = spyOn(globalThis, "setInterval").mockImplementation(((_fn: () => void, ms?: number) => {
		intervals.push(ms ?? 0);
		return 0;
	}) as unknown as typeof setInterval);
	restores.push(() => later.mockRestore(), () => cancel.mockRestore(), () => every.mockRestore());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	const container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	for (const restore of restores.reverse()) restore();
	Reflect.deleteProperty(document, "visibilityState");
	await teardown();
});

let renders = 0;
let seen: string | undefined;
async function mount(nudge?: string) {
	const { useComputerViewer } = await import("../src/computer");
	function Key({ nudge }: { nudge?: string | undefined }) {
		renders++;
		seen = useComputerViewer("ada", true, nudge);
		return null;
	}
	renders = 0;
	await act(async () => root.render(<Key nudge={nudge} />));
	return (next?: string) => act(async () => root.render(<Key nudge={next} />));
}
const delays = () => [...timers.values()].map((one) => one.ms);
async function tick() {
	await act(async () => {
		for (const [id, one] of [...timers]) {
			timers.delete(id);
			one.fn();
		}
	});
}

test("behind another app, an idle conversation asks after its computer once a minute and never every 5 s, and an answer that has not moved draws nothing", async () => {
	status = { state: "running", viewer: "http://127.0.0.1:6080/" };
	await mount();
	expect(asked).toBe(1);
	expect(seen).toBe("http://127.0.0.1:6080/");
	expect(intervals).toEqual([]);
	expect(delays()).toEqual([60_000]);
	const settled = renders;
	for (let i = 0; i < 5; i++) await tick();
	expect(asked).toBe(6);
	expect(delays()).toEqual([60_000]);
	expect(renders).toBe(settled);
	// A computer that stopped on its own is still seen, at the next look.
	status = { state: "stopped" };
	await tick();
	expect(seen).toBeUndefined();
});

test("in front it asks every 10 s; coming in front or into view asks at once; out of view it asks nothing", async () => {
	status = { state: "stopped" };
	focused = true;
	await mount();
	expect(delays()).toEqual([10_000]);
	await act(async () => {
		visibility = "hidden";
		document.dispatchEvent(new window.Event("visibilitychange"));
	});
	expect(delays()).toEqual([]);
	await tick();
	expect(asked).toBe(1);
	status = { state: "running", viewer: "http://127.0.0.1:6080/" };
	await act(async () => {
		visibility = "visible";
		document.dispatchEvent(new window.Event("visibilitychange"));
	});
	expect(asked).toBe(2);
	expect(seen).toBe("http://127.0.0.1:6080/");
	focused = false;
	await act(async () => window.dispatchEvent(new window.Event("blur")));
	status = { state: "stopped" };
	focused = true;
	await act(async () => window.dispatchEvent(new window.Event("focus")));
	expect(asked).toBe(3);
	expect(seen).toBeUndefined();
	expect(delays()).toEqual([10_000]);
});

test("a turn that moves asks again at once, so a computer the teammate started in it is seen without waiting for the slow tick", async () => {
	status = { state: "stopped" };
	const rerender = await mount("working|true");
	expect(asked).toBe(1);
	await rerender("working|true");
	expect(asked).toBe(1);
	status = { state: "running", viewer: "http://127.0.0.1:6080/" };
	await rerender("idle|true");
	expect(asked).toBe(2);
	expect(seen).toBe("http://127.0.0.1:6080/");
	expect(delays()).toEqual([60_000]);
});
