import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";

// Loading the window's modules reads the platform and the media queries it
// watches from `window`, before any test has a DOM.
const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });
const { App } = await import("../src/App");
const { replaceDesks } = await import("../src/desks");

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

/** What the fake core says `welcome` is; everything else it answers empty. */
let setUp: boolean;

/**
 * A core on the other end of the window's socket: it takes the hello,
 * answers every view with an empty snapshot (an empty roster among them),
 * `welcome` with `setUp`, and refuses whatever else is asked.
 */
class FakeCore {
	static readonly OPEN = 1;
	static all: FakeCore[] = [];
	readyState = 1;
	sent: Record<string, unknown>[] = [];
	onopen: (() => void) | null = null;
	onmessage: ((message: { data: string }) => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	constructor(readonly url: string) {
		FakeCore.all.push(this);
		setTimeout(() => this.onopen?.(), 0);
	}
	send(data: string) {
		const frame = JSON.parse(data) as Record<string, unknown>;
		this.sent.push(frame);
		const id = frame["id"];
		const reply = (answer: Record<string, unknown>) => setTimeout(() => this.onmessage?.({ data: JSON.stringify(answer) }), 0);
		if ("sub" in frame) {
			reply({ id, ok: true });
			reply({ sub: id, snapshot: [] });
		} else if (frame["cmd"] === "client.hello") {
			reply({ id, ok: true, result: { capabilities: ["threads", "threads2"] } });
		} else if (frame["cmd"] === "welcome") {
			reply({ id, ok: true, result: { providers: [], harnesses: [], defaultBackendId: "hotline", canRun: false, teammates: 0, setUp } });
		} else if ("cmd" in frame) {
			reply({ id, ok: false, error: "Not in this test." });
		}
	}
	close() {
		if (this.readyState === 3) return;
		this.readyState = 3;
		this.onclose?.();
	}
}

let act: typeof import("react").act;
let root: Root | null;
let container: HTMLDivElement;
let dom: Window;
let restores: (() => void)[];

beforeEach(async () => {
	restores = [];
	dom = new Window();
	FakeCore.all = [];
	const globals = {
		window: dom,
		document: dom.document,
		navigator: dom.navigator,
		localStorage: dom.localStorage,
		IS_REACT_ACT_ENVIRONMENT: true,
		ResizeObserver: NoResize,
		WebSocket: FakeCore,
	};
	for (const [key, value] of Object.entries(globals)) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		});
	}
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "ws://core", token: "t" }]);
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

/** Opens the window and lets the hello, the roster and `welcome` land. */
async function open() {
	await act(async () => {
		root!.render(<App />);
	});
	for (let turn = 0; turn < 5; turn++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 5));
		});
	}
}
const text = () => container.textContent ?? "";
const asked = (cmd: string) => FakeCore.all.some((socket) => socket.sent.some((frame) => frame["cmd"] === cmd));

describe("an empty room", () => {
	test("set up before, it keeps the window and offers New teammate instead of the welcome", async () => {
		setUp = true;
		await open();
		expect(asked("welcome")).toBe(true);
		expect(text()).toContain("No teammates yet");
		expect(text()).toContain("Add one to start a conversation.");
		expect(text()).not.toContain("Welcome to Hotline");
		// The window keeps its rail: this is the room, not a screen in front of it.
		expect(container.querySelector('nav[aria-label="Team"]')).not.toBeNull();

		const button = [...container.querySelectorAll<HTMLButtonElement>(".btn-primary")].find((node) => node.textContent?.includes("New teammate"));
		expect(button).toBeDefined();
		await act(async () => {
			button!.click();
		});
		// The pane is the plus's own form, in the conversation's place.
		expect(text()).toContain("An AI helper with a name and a job.");
		expect(text()).not.toContain("Add one to start a conversation.");
	});

	test("never set up, it is the welcome alone", async () => {
		setUp = false;
		await open();
		expect(text()).toContain("Welcome to Hotline");
		expect(text()).not.toContain("Add one to start a conversation.");
		expect(container.querySelector('nav[aria-label="Team"]')).toBeNull();
	});
});
