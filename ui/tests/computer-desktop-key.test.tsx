import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { ComputerStatus } from "../src/generated/contract";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { ComputerRows } = await import("../src/components/teammate/ComputerRows");
const { wire } = await import("../src/wire");

let calls: string[];
let status: ComputerStatus;
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;

beforeEach(async () => {
	restores = [];
	({ teardown } = installDom());
	calls = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string) => {
		calls.push(cmd);
		if (cmd === "computer.status") return status;
		if (cmd === "computer.capacity") return { runtime: "docker", cpus: 8, memoryBytes: 16 * 1024 ** 3, source: "runtime" };
		if (cmd === "computer.start") {
			status = { state: "running" };
			return null;
		}
		if (cmd === "computer.stop") {
			status = { state: "stopped" };
			return null;
		}
		throw new Error(`Not in this test: ${cmd}`);
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	for (const restore of restores.reverse()) restore();
	await teardown();
});

const render = (enabled: boolean) =>
	act(async () => {
		root.render(<ComputerRows personaId="ada" teammate="Ada" computer={{ enabled }} disabled={false} onChange={() => {}} />);
	});
const key = (label: string) => container.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`);
const fold = () => container.querySelector<HTMLButtonElement>(".fold-row-open");

test("the desktop row starts a stopped computer and stops a running one, before its chevron, without opening", async () => {
	status = { state: "stopped" };
	await render(true);
	const start = key("Start the computer");
	expect(start).not.toBeNull();
	// The key stands between the row's text and its chevron.
	const row = start!.parentElement!;
	expect([...row.children].map((child) => child.className.split(" ").find((name) => name.startsWith("fold-row-") || name === "control"))).toEqual(["fold-row-open", "control", "fold-row-chevron"]);
	await act(async () => start!.click());
	expect(calls).toContain("computer.start");
	expect(fold()?.getAttribute("aria-expanded")).toBe("false");
	expect(fold()?.textContent).toContain("running");
	const stop = key("Stop the computer");
	expect(stop).not.toBeNull();
	await act(async () => stop!.click());
	expect(calls).toContain("computer.stop");
	expect(fold()?.textContent).toContain("stopped");
	// The text still opens the row.
	await act(async () => fold()!.click());
	expect(fold()?.getAttribute("aria-expanded")).toBe("true");
});

test("a computer whose grant is off is not started from the row, but one still running can be stopped", async () => {
	status = { state: "stopped" };
	await render(false);
	expect(key("Start the computer")).toBeNull();
	status = { state: "running" };
	await act(async () => root.unmount());
	root = (await import("react-dom/client")).createRoot(container);
	await render(false);
	expect(key("Stop the computer")).not.toBeNull();
});
