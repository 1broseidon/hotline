import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { SessionProcess } from "../src/generated/contract";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { processLabel, shownState } = await import("../src/activity");

const server: SessionProcess = { id: "call_serve", title: "cd site && python3 -m http.server 8770", startedAt: 1, state: "running" };
const watcher: SessionProcess = { id: "call_watch", title: "bun run dev", startedAt: 2, state: "running" };

describe("a running process's label", () => {
	test("is the command past its cd and its interpreter", () => {
		expect(processLabel(server.title)).toBe("http.server 8770");
		expect(processLabel("echo $$ > server.pid; exec python3 -m http.server 0")).toBe("http.server 0");
		expect(processLabel("bun run dev")).toBe("bun run dev");
		expect(processLabel(";")).toBe(";");
	});

	test("never changes the session's state", () => {
		expect(shownState({ state: "ready" })).toBe("ready");
		expect(shownState({ state: "idle" })).toBe("idle");
	});
});

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let dom: Window;
let restores: (() => void)[];

beforeEach(async () => {
	dom = new Window();
	restores = [];
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, localStorage: dom.localStorage, IS_REACT_ACT_ENVIRONMENT: true })) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => (descriptor ? Object.defineProperty(globalThis, key, descriptor) : Reflect.deleteProperty(globalThis, key)));
	}
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	for (const restore of restores.reverse()) restore();
	await dom.happyDOM.close();
});

describe("the processes at the composer's corner", () => {
	test("name one by its short command and open its card", async () => {
		const { ProcessChips } = await import("../src/components/Conversation");
		const opened: string[] = [];
		await act(async () => {
			root.render(<ProcessChips processes={[server]} opened={(id) => id === "call_serve"} onOpen={(id) => opened.push(id)} />);
		});
		const chip = container.querySelector<HTMLButtonElement>(`[aria-label="Running process: ${server.title}"]`);
		expect(chip?.textContent).toBe("http.server 8770");
		expect(chip?.getAttribute("title")).toBe("Running process");
		expect(chip?.getAttribute("aria-pressed")).toBe("true");
		await act(async () => chip!.click());
		expect(opened).toEqual(["call_serve"]);
	});

	test("count several, with a menu that opens any of them", async () => {
		const { ProcessChips } = await import("../src/components/Conversation");
		const opened: string[] = [];
		await act(async () => {
			root.render(<ProcessChips processes={[server, watcher]} opened={() => false} onOpen={(id) => opened.push(id)} />);
		});
		const chip = container.querySelector<HTMLButtonElement>('[aria-label="2 processes running"]');
		expect(chip?.textContent).toBe("2 processes");
		expect(chip?.getAttribute("title")).toBe("Running processes");
		await act(async () => chip!.click());
		const items = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
		expect(items.length).toBe(2);
		await act(async () => items[1]!.click());
		expect(opened).toEqual(["call_watch"]);
	});
});
