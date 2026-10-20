import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { ScheduleEntry } from "../src/generated/contract";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { JobActions } = await import("../src/components/Dock");
const { wire } = await import("../src/wire");

const sweep = { id: "sweep", personaId: "ada", name: "Sweep the inbox", kind: "loop", prompt: "Sweep the inbox", nextAt: 0 } as ScheduleEntry;

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let sent: { cmd: string; params: unknown }[];
let refusal: string | null;
let restore: () => void;
let teardown: () => Promise<void>;

beforeEach(async () => {
	({ teardown } = installDom());
	sent = [];
	refusal = null;
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		sent.push({ cmd, params });
		if (refusal !== null) throw new Error(refusal);
		return null;
	}) as typeof wire.command);
	restore = () => command.mockRestore();
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	restore();
	await teardown();
});

async function show(running: boolean): Promise<(string | null)[]> {
	const said: (string | null)[] = [];
	await act(async () => root.render(<JobActions job={sweep} running={running} onSaid={(one) => said.push(one)} />));
	return said;
}

async function press(label: string) {
	const button = container.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`);
	if (button === null) throw new Error(`No ${label} in: ${container.innerHTML}`);
	await act(async () => button.click());
}

describe("a job's Stop and Cancel", () => {
	test("Cancel ends the job by closing its thread", async () => {
		const said = await show(false);
		await press("Cancel Sweep the inbox");
		expect(sent).toEqual([{ cmd: "thread.close", params: { thread: { kind: "job", key: "sweep" } } }]);
		expect(said).toEqual([null]);
	});

	test("Stop is there only while a turn runs, and stops that turn alone", async () => {
		await show(false);
		expect(container.querySelector('[aria-label="Stop Sweep the inbox"]')).toBeNull();
		const said = await show(true);
		await press("Stop Sweep the inbox");
		expect(sent).toEqual([{ cmd: "thread.cancel", params: { thread: { kind: "job", key: "sweep" } } }]);
		expect(said).toEqual([null]);
	});

	test("say what the desk refused", async () => {
		refusal = "That job has no turn running.";
		const said = await show(true);
		await press("Stop Sweep the inbox");
		expect(said).toEqual(["That job has no turn running."]);
	});
});
