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

function primary() {
	return container.querySelector<HTMLButtonElement>(".btn-primary");
}
async function pick(title: string) {
	const row = [...container.querySelectorAll("label")].find((node) => node.textContent?.startsWith(title));
	if (!row) throw new Error(`Missing choice ${title}: ${text()}`);
	await act(async () => {
		row.querySelector("input")!.click();
	});
}

describe("the greeting on this computer's own desk", () => {
	test("says what Hotline is, with one way forward and a server as the quieter way in", async () => {
		await mount(connect);
		expect(text()).toContain("Welcome to Hotline");
		expect(primary()?.textContent).toBe("Get started");
		expect(button("Connect to a server instead").className).not.toContain("btn-primary");
		// Nothing is asked yet.
		expect(text()).not.toContain("Setup steps");
		expect(container.querySelector("ol")).toBeNull();
	});

	test("Connect to a server instead opens the Add a server pane and leaves the greeting up", async () => {
		await mount(connect);
		await click("Connect to a server instead");
		expect(connected).toBe(1);
		expect(text()).toContain("Welcome to Hotline");
	});

	test("Get started goes to why Hotline, and Back returns to the greeting", async () => {
		await mount(connect);
		await click("Get started");
		expect(text()).toContain("Why Hotline");
		for (const step of ["Why Hotline", "How they think", "Connect", "Your teammate"]) expect(text()).toContain(step);
		expect(connected).toBe(0);
		await click("Back");
		expect(text()).toContain("Welcome to Hotline");
	});
});

describe("the wizard", () => {
	test("states the four pillars before anything is asked", async () => {
		await mount(null);
		await click("Get started");
		for (const pillar of ["Runs on your computer", "Bring your own AI", "A computer of their own", "With you anywhere"]) expect(text()).toContain(pillar);
		expect(primary()?.disabled).toBe(false);
	});

	test("names the two ways to think and waits for one before going on", async () => {
		await mount(null);
		await click("Get started");
		await click("Continue");
		expect(text()).toContain("How your teammates think");
		expect(text()).toContain("Hotline Agent");
		expect(text()).toContain("Recommended");
		expect(text()).toContain("An AI coding tool you already have");
		expect(primary()?.disabled).toBe(true);
		await pick("Hotline Agent");
		expect(primary()?.disabled).toBe(false);
		await click("Back");
		expect(text()).toContain("Why Hotline");
	});

	test("cannot pick a tool this machine does not have", async () => {
		await mount(null);
		await click("Get started");
		await click("Continue");
		const already = [...container.querySelectorAll("label")].find((node) => node.textContent?.startsWith("An AI coding tool you already have"));
		expect(already?.querySelector("input")?.disabled).toBe(true);
		expect(text()).toContain("None is on this computer");
	});
});

describe("the greeting on a server's own desk", () => {
	test("has nothing to choose between: no server to connect to", async () => {
		await mount(null);
		expect(text()).toContain("Welcome to Hotline");
		expect(text()).not.toContain("Connect to a server");
	});
});
