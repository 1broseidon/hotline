import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";

// Loading the window's modules reads the platform from `window`, before any test has a DOM.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Welcome } = await import("../src/components/Welcome");
const { replaceDesks } = await import("../src/desks");

// No socket or shell: the desk has no endpoint, so its wire never opens and Welcome reads nothing.
class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

let act: typeof import("react").act;
let root: Root | null;
let container: HTMLDivElement;
let dom: Window;
let restores: (() => void)[];
let connected: number;
let created: string[];

beforeEach(async () => {
	restores = [];
	dom = new Window();
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, IS_REACT_ACT_ENVIRONMENT: true, ResizeObserver: NoResize })) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		});
	}
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	connected = 0;
	created = [];
	// React reads the DOM when it loads, so it loads once the DOM is here (as search.test.tsx does).
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
	await dom.happyDOM.close();
});

async function mount(onConnectServer: (() => void) | null) {
	await act(async () => {
		root!.render(<Welcome models={[]} onCreated={(id) => created.push(id)} onConnectServer={onConnectServer} />);
	});
}
const text = () => container.textContent ?? "";
function button(label: string) {
	const found = [...container.querySelectorAll("button")].find((node) => node.textContent?.includes(label));
	if (!found) throw new Error(`Missing button ${label}: ${text()}`);
	return found;
}
async function click(label: string) {
	await act(async () => {
		button(label).click();
	});
}
const connect = () => {
	connected += 1;
};

describe("the setup screen on this computer's own desk", () => {
	test("offers creating a first teammate and connecting to a server as two equal choices, one line each", async () => {
		await mount(connect);
		expect(text()).toContain("Where your teammates run");
		expect(text()).toContain("Create a first teammate");
		expect(text()).toContain("Teammates on this computer.");
		expect(text()).toContain("Connect to a server");
		expect(text()).toContain("Teammates on a server you run.");
		// The steps wait for the choice, and neither choice is the screen's primary.
		expect(text()).not.toContain("A way to run agents");
		expect(container.querySelector(".btn-primary")).toBeNull();
		const rows = [...container.querySelectorAll("button")];
		expect(rows.map((row) => row.className)).toEqual([rows[0]!.className, rows[0]!.className]);
	});

	test("Connect to a server opens the Add a server pane and keeps the choice on screen", async () => {
		await mount(connect);
		await click("Connect to a server");
		expect(connected).toBe(1);
		expect(text()).toContain("Where your teammates run");
	});

	test("Create a first teammate goes on to the steps, and Connect to a server stays one press away", async () => {
		await mount(connect);
		await click("Create a first teammate");
		expect(text()).toContain("A way to run agents");
		expect(text()).toContain("Your first teammate");
		expect(text()).toContain("Say hello");
		expect(text()).not.toContain("Where your teammates run");
		expect(connected).toBe(0);
		await click("Connect to a server");
		expect(connected).toBe(1);
	});
});

describe("the setup screen on a server's own desk", () => {
	test("goes straight to the steps: there is nothing to choose between", async () => {
		await mount(null);
		expect(text()).toContain("A way to run agents");
		expect(text()).not.toContain("Where your teammates run");
		expect(text()).not.toContain("Connect to a server");
	});
});
