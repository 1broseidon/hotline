import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from "bun:test";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

// Loading the window's modules reads the platform, and the theme, before any test has a DOM.
Object.assign(globalThis, {
	window: { matchMedia: () => ({ matches: false, addEventListener() {} }) },
	localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
});

let opened: string[] = [];
let refuse = false;
const native = await import("../src/native");
mock.module("../src/native", () => ({
	...native,
	openAccountSetup: async (path: string) => {
		opened.push(path);
		if (refuse) throw new Error("That is not part of an agent account's setup.");
	},
}));
const { wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");
const { AccountSetup } = await import("../src/components/settings/AccountSetup");
Reflect.deleteProperty(globalThis, "localStorage");

const at = Date.UTC(2026, 9, 21, 12);
const dir = "/data/agent-accounts/a1";
const setup = {
	setup: { from: "system_default", at, categories: ["instructions", "skills", "plugins", "mcp", "settings"] },
	categories: [
		{ id: "instructions", count: 2, path: `${dir}/CLAUDE.md` },
		{ id: "skills", count: 6, path: `${dir}/skills/` },
		{ id: "plugins", count: 3, path: `${dir}/plugins/` },
		{ id: "mcp", count: 2, path: `${dir}/.claude.json` },
		{ id: "settings", count: 1, path: `${dir}/settings.json` },
	],
};
const local = { id: "local", name: "This computer", kind: "local" as const, origin: "", token: "" };
const remote = { id: "server-1", name: "Studio", kind: "remote" as const, origin: "", token: "" };

type Call = [string, Record<string, unknown>];
let calls: Call[];
let root: Root | null;
let container: HTMLDivElement;
let dom: Window;
let act: typeof import("react").act;
let restores: (() => void)[];
let teardown: () => Promise<void>;

beforeEach(async () => {
	restores = [];
	({ dom, teardown } = installDom());
	(dom as unknown as { __hotlineDesk: unknown }).__hotlineDesk = { platform: "windows" };
	replaceDesks([local]);
	opened = [];
	refuse = false;
	calls = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push([cmd, params]);
		if (cmd === "accounts.setup.list") return setup;
		if (cmd === "accounts.setup.copy") return { copied: params["categories"], removedKeys: [], linksSkipped: 0 };
		throw new Error(`Unexpected command ${cmd}`);
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => {
		root?.unmount();
		root = null;
	});
	replaceDesks([]);
	for (const restore of restores.reverse()) restore();
	await teardown();
});

async function mount() {
	await act(async () => {
		root!.render(<AccountSetup accountId="a1" agentName="Claude Code" />);
	});
	await settle();
}
async function settle() {
	for (let round = 0; round < 4; round++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
	}
}
const text = () => container.textContent ?? "";
const titles = () => [...container.querySelectorAll(".group-row-title")].map((node) => node.textContent);
const opens = () => [...container.querySelectorAll("button")].filter((node) => node.textContent === "Open");
function button(label: string) {
	const found = [...container.querySelectorAll("button")].find((node) => node.textContent === label || node.getAttribute("aria-label") === label);
	if (!found) throw new Error(`Missing button ${label}: ${text()}`);
	return found as HTMLButtonElement;
}
async function click(label: string) {
	const found = button(label);
	await act(async () => {
		found.click();
	});
	await settle();
}
function box(title: string): HTMLInputElement {
	const found = [...container.querySelectorAll("label")].find((node) => node.textContent?.startsWith(title));
	if (!found) throw new Error(`Missing checkbox ${title}: ${text()}`);
	return found.querySelector('input[type="checkbox"]') as HTMLInputElement;
}
const said = () => [...container.querySelectorAll('[role="status"]')].map((node) => node.textContent ?? "");

describe("an agent account's setup", () => {
	test("lists each item with its count under where it started", async () => {
		await mount();
		expect(calls).toEqual([["accounts.setup.list", { accountId: "a1" }]]);
		expect(text()).toContain("Started from your Claude Code setup on");
		expect(text()).toContain("6 skills");
		expect(text()).toContain("3 plugins");
		expect(text()).toContain("2 servers");
		expect(text()).not.toContain("CLAUDE.md");
		expect(text()).not.toContain("settings.json");
		expect(titles().slice(0, 5)).toEqual(["Instructions", "Settings", "Skills", "Plugins", "MCP servers"]);
		expect(text()).toContain("Opens in your own editor or file manager on this computer. Changes apply from the next session of the teammates on this account.");
	});

	test("Open stands only on the Desk seat and opens the item through the shell", async () => {
		await mount();
		expect(opens()).toHaveLength(5);
		await click("Open Instructions");
		expect(opened).toEqual([`${dir}/CLAUDE.md`]);
		expect(said()).toContain("CLAUDE.md opened in your editor.");
		await click("Open Skills");
		expect(opened.at(-1)).toBe(`${dir}/skills/`);
		expect(said()).toContain("The skills folder opened.");
		refuse = true;
		await click("Open Settings");
		expect(text()).toContain("That is not part of an agent account's setup.");

		await act(async () => {
			root?.unmount();
			const { createRoot } = await import("react-dom/client");
			root = createRoot(container);
		});
		replaceDesks([remote]);
		await mount();
		expect(text()).toContain("6 skills");
		expect(opens()).toHaveLength(0);
	});

	test("Copy again ticks all but Settings and copies the ticked", async () => {
		await mount();
		expect(text()).toContain("Bring over what you changed in your own Claude Code setup since this account was made");
		await click("Choose…");
		expect(box("Settings").checked).toBe(false);
		for (const title of ["Instructions and rules", "Skills, agents and commands", "Plugins", "MCP servers"]) expect(box(title).checked).toBe(true);
		expect(text()).toContain("The ticked items replace this account's, including your changes to them. The sign-in is not touched.");
		await click("Copy 4 items");
		expect(calls).toContainEqual(["accounts.setup.copy", { accountId: "a1", categories: ["instructions", "skills", "plugins", "mcp"] }]);
		expect(said()).toContain("Copied 4 items from This computer.");
		expect(calls.filter(([cmd]) => cmd === "accounts.setup.list")).toHaveLength(2);
		expect(text()).toContain("The sign-in, keys and history are never copied.");
	});
});
