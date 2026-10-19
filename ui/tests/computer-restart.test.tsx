import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { ComputerStatus } from "../src/generated/contract";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { ComputerRows } = await import("../src/components/teammate/ComputerRows");
const { wire } = await import("../src/wire");

type Call = { cmd: string; params: Record<string, unknown> };
let calls: Call[];
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
	status = { state: "running" };
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push({ cmd, params });
		if (cmd === "computer.status") return status;
		if (cmd === "computer.capacity") return { runtime: "docker", cpus: 8, memoryBytes: 16 * 1024 ** 3, source: "runtime" };
		if (cmd === "computer.restart") {
			status = { state: "running" };
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

async function openDesktop() {
	await act(async () => {
		root.render(<ComputerRows personaId="ada" teammate="Ada" computer={{ enabled: true, memory: "2048m" }} disabled={false} onChange={() => {}} />);
	});
	const fold = [...container.querySelectorAll("button")].find((node) => node.textContent?.includes("Desktop"));
	if (!fold) throw new Error("No Desktop fold");
	await act(async () => fold.click());
}

const restart = () => [...container.querySelectorAll("button")].find((node) => node.textContent === "Restart to apply");

describe("a computer's memory and CPUs", () => {
	test("say they apply at the next start, with no restart while the running computer has them", async () => {
		await openDesktop();
		expect(container.textContent).toContain("Applies the next time the computer starts.");
		expect(restart()).toBeUndefined();
	});

	test("offer a restart while the running computer has others, which asks the desk for one", async () => {
		status = { state: "running", limitsChanged: true };
		await openDesktop();
		const button = restart();
		if (!button) throw new Error("No Restart to apply");
		await act(async () => button.click());
		expect(calls.some((call) => call.cmd === "computer.restart" && call.params["personaId"] === "ada")).toBe(true);
		expect(restart()).toBeUndefined();
	});
});
