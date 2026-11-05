import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import { act } from "react";
import type { Root } from "react-dom/client";
import type { WebSearchProviderStatus } from "../src/generated/contract";
import { installDom } from "./dom";

// React decides at load whether this DOM has `input` events, so it loads with one in place.
const observer = { ResizeObserver: class { observe() {} unobserve() {} disconnect() {} } };
const boot = installDom(observer);
const { createRoot } = await import("react-dom/client");
const { ToolsSection } = await import("../src/components/settings/Tools");
const { wire } = await import("../src/wire");
await boot.teardown();

const KEY = "searxng-key-never-drawn-0123";

let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
/** What the four web search commands take, together. */
type Params = { provider?: string; url?: string; key?: string; enabled?: boolean };

let sent: { cmd: string; params: Params }[];
let providers: WebSearchProviderStatus[];
let restore: () => void;

const hosted = (provider: "parallel" | "exa", name: string): WebSearchProviderStatus => ({ provider, name, enabled: true, hasKey: false });

/** The desk's side of the four web search commands, as the core answers them. */
function desk(cmd: string, params: Params) {
	const searxng = providers.find((one) => one.provider === "searxng")!;
	if (cmd === "websearch.set_url") {
		if (typeof params.url === "string" && !params.url.startsWith("http")) throw new Error("Enter the SearXNG instance's full http or https address.");
		if (params.url === undefined) {
			delete searxng.url;
			searxng.enabled = false;
		} else searxng.url = params.url;
	}
	if (cmd === "websearch.set_key") searxng.hasKey = params.key !== undefined;
	if (cmd === "websearch.set_enabled") searxng.enabled = params.enabled === true;
	return { providers: providers.map((one) => ({ ...one })) };
}

beforeEach(() => {
	({ teardown } = installDom(observer));
	sent = [];
	providers = [hosted("parallel", "Parallel"), hosted("exa", "Exa"), { provider: "searxng", name: "SearXNG", enabled: false, hasKey: false, takesUrl: true }];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Params) => {
		sent.push({ cmd, params });
		return desk(cmd, params);
	}) as unknown as typeof wire.command);
	restore = () => command.mockRestore();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	restore();
	await teardown();
});

const settle = () =>
	act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});

async function show() {
	await act(async () => root.render(<ToolsSection servers={[]} />));
	await settle();
}

function type(field: HTMLInputElement, text: string) {
	const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(field), "value")!.set!;
	setter.call(field, text);
	field.dispatchEvent(new window.Event("input", { bubbles: true }));
}

function row(name: string): HTMLElement {
	return [...container.querySelectorAll<HTMLElement>(".group-row")].find((one) => one.querySelector(".group-row-title")?.textContent === name)!;
}

const button = (within: Element, label: string) => [...within.querySelectorAll("button")].find((one) => one.textContent === label);
const searxngSwitch = () => row("SearXNG").querySelector<HTMLInputElement>('input[role="switch"]')!;

test("SearXNG is listed off with no instance, and its switch waits for a URL", async () => {
	await show();
	expect(row("SearXNG").querySelector(".group-row-detail")?.textContent).toBe("No instance set");
	expect(searxngSwitch().checked).toBe(false);
	expect(searxngSwitch().disabled).toBe(true);
	expect(searxngSwitch().title).toBe("Set URL first");
	expect(button(row("SearXNG"), "Set up")).toBeDefined();
	// The hosted rows keep their key action and have no URL.
	expect(button(row("Exa"), "Add key")).toBeDefined();
	expect(row("Exa").querySelector(".group-row-detail")?.textContent).toBe("Keyless");
});

test("Set up saves the URL and an optional key, the key never drawn back, then the switch turns it on", async () => {
	await show();
	await act(async () => button(row("SearXNG"), "Set up")!.click());
	const url = container.querySelector<HTMLInputElement>("#searxng-url")!;
	const key = container.querySelector<HTMLInputElement>("#searxng-key")!;
	expect(key.type).toBe("password");
	expect(key.placeholder).toBe("Optional");
	await act(async () => type(url, " http://localhost:8888 "));
	await act(async () => type(key, KEY));
	await act(async () => button(container, "Save")!.click());
	await settle();
	expect(sent.filter((one) => one.cmd !== "websearch.status")).toEqual([
		{ cmd: "websearch.set_url", params: { provider: "searxng", url: "http://localhost:8888" } },
		{ cmd: "websearch.set_key", params: { provider: "searxng", key: KEY } },
	]);
	expect(container.querySelector("#searxng-url")).toBeNull();
	expect(container.innerHTML).not.toContain(KEY);
	expect(row("SearXNG").querySelector(".group-row-detail")?.textContent).toBe("http://localhost:8888");
	expect(searxngSwitch().disabled).toBe(false);

	await act(async () => searxngSwitch().click());
	expect(sent.at(-1)).toEqual({ cmd: "websearch.set_enabled", params: { provider: "searxng", enabled: true } });
	expect(searxngSwitch().checked).toBe(true);
});

test("a refused URL keeps the form open with the desk's reason, and a blank one takes the instance away", async () => {
	providers[2] = { ...providers[2]!, url: "http://localhost:8888", enabled: true };
	await show();
	await act(async () => button(row("SearXNG"), "Edit")!.click());
	const url = container.querySelector<HTMLInputElement>("#searxng-url")!;
	expect(url.value).toBe("http://localhost:8888");
	await act(async () => type(url, "localhost:9999"));
	await act(async () => button(container, "Save")!.click());
	await settle();
	expect(container.querySelector("#searxng-url")).not.toBeNull();
	expect(container.textContent).toContain("full http or https address");

	await act(async () => type(container.querySelector<HTMLInputElement>("#searxng-url")!, ""));
	await act(async () => button(container, "Save")!.click());
	await settle();
	expect(sent.at(-1)).toEqual({ cmd: "websearch.set_url", params: { provider: "searxng" } });
	expect(searxngSwitch().checked).toBe(false);
	expect(searxngSwitch().disabled).toBe(true);
});
