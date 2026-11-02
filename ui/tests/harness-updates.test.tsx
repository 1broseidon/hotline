import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { HarnessUpdate, HarnessUpdates } from "../src/generated/contract";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { HarnessSection, harnessValue } = await import("../src/components/UpdatesSection");
const { replaceDesks, wireFor } = await import("../src/desks");

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let restore: () => void;
let sent: { cmd: string; params: unknown }[];
let listed: HarnessUpdates;
let answer: (backendId: string) => Promise<HarnessUpdate>;

const claude: HarnessUpdate = { backendId: "claude-acp", name: "Claude Code", state: "available", version: "2.1.3", latest: "2.1.5", canUpdate: true };
const codex: HarnessUpdate = { backendId: "codex-acp", name: "Codex", state: "upToDate", version: "0.5.0", canUpdate: false };

beforeEach(async () => {
	({ teardown } = installDom());
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	sent = [];
	listed = { harnesses: [claude, codex], checkedAt: 1_793_000_000_000 };
	answer = async () => ({ ...claude, state: "updated", version: "2.1.5", canUpdate: false, detail: "Claude Code is 2.1.5. A teammate on it gets it at its next start." });
	const command = spyOn(wireFor("local"), "command").mockImplementation((async (cmd: string, params: { backendId: string }) => {
		sent.push({ cmd, params });
		if (cmd === "harness.updates" || cmd === "harness.updates.check") return listed;
		if (cmd === "harness.update") return answer(params.backendId);
		throw new Error(`Not in this test: ${cmd}`);
	}) as Wire["command"]);
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
	container.remove();
	await teardown();
});

const mount = async () => {
	await act(async () => root.render(<HarnessSection />));
	await act(async () => {});
};
const row = (name: string) => container.querySelector(`[aria-label="${name}"]`) as HTMLElement;
const buttonIn = (scope: Element, label: string) =>
	[...scope.querySelectorAll("button")].find((button) => button.textContent === label) as HTMLButtonElement | undefined;

test("each harness is one row, and only one that is behind offers Update", async () => {
	await mount();
	expect(row("Claude Code").textContent).toContain("Claude Code 2.1.3");
	expect(row("Claude Code").textContent).toContain("2.1.5 is available");
	expect(buttonIn(row("Claude Code"), "Update")).toBeDefined();
	expect(row("Codex").textContent).toContain("Up to date");
	expect(buttonIn(row("Codex"), "Update")).toBeUndefined();
	expect(sent.map((one) => one.cmd)).toEqual(["harness.updates"]);
});

test("Update runs on the desk, shows it is running, and then says when it takes effect", async () => {
	let finish: (row: HarnessUpdate) => void = () => {};
	answer = () => new Promise((resolve) => (finish = resolve));
	await mount();
	await act(async () => buttonIn(row("Claude Code"), "Update")!.click());
	expect(row("Claude Code").textContent).toContain("Updating…");
	expect(buttonIn(row("Claude Code"), "Update")).toBeUndefined();
	expect(buttonIn(container, "Check now")!.disabled).toBe(true);
	expect(sent.at(-1)).toEqual({ cmd: "harness.update", params: { backendId: "claude-acp" } });
	await act(async () => finish({ ...claude, state: "updated", version: "2.1.5", canUpdate: false, detail: "Claude Code is 2.1.5. A teammate on it gets it at its next start." }));
	expect(row("Claude Code").textContent).toContain("Claude Code 2.1.5");
	expect(row("Claude Code").textContent).toContain("gets it at its next start");
	expect(buttonIn(container, "Check now")!.disabled).toBe(false);
});

test("a refused update says why and keeps the row", async () => {
	answer = async () => {
		throw new Error("Claude Code is already being updated.");
	};
	await mount();
	await act(async () => buttonIn(row("Claude Code"), "Update")!.click());
	expect(row("Claude Code").textContent).toContain("Update failed");
	await act(async () => buttonIn(row("Claude Code"), "Details")!.click());
	expect(row("Claude Code").textContent).toContain("already being updated");
	expect(buttonIn(row("Claude Code"), "Update")).toBeDefined();
});

test("a desk never checked looks at once rather than show nothing", async () => {
	listed = { harnesses: [] };
	await mount();
	expect(sent.map((one) => one.cmd)).toEqual(["harness.updates", "harness.updates.check"]);
});

test("every state reads as a few words", () => {
	const words = (state: HarnessUpdate["state"], extra: Partial<HarnessUpdate> = {}) => harnessValue({ ...codex, ...extra, state });
	expect(words("available")).toBe("Update available");
	expect(words("available", { latest: "0.6.0" })).toBe("0.6.0 is available");
	expect(words("unknown")).toBe("Can't tell if it's current");
	expect(words("checking")).toBe("Checking…");
	expect(words("failed")).toBe("Update failed");
});
