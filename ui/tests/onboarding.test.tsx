import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

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

const entry = (id: string, name: string) => ({
	persona: {
		id,
		name,
		goal: "Help me get things done.",
		backendId: "hotline",
		cwd: "/work",
		mcpPolicy: { mode: "all", serverIds: [] },
		skillPolicy: { mode: "all", names: [] },
		backgroundWork: false,
		allowedSenders: [],
		sessionCheckpoints: [],
		createdAt: 0,
		updatedAt: 0,
	},
	session: {
		personaId: id,
		state: "idle",
		contextRestored: false,
		models: [],
		modes: [],
		configs: [],
		slashCommands: [],
		capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
	},
	waiting: false,
	drawing: false,
});

/**
 * A fresh room that can already run, so the welcome opens on its last
 * step. `persona.create` puts the new teammate in the roster the way the
 * core does, by an event on the roster subscription before the answer.
 * `session.start` is answered only when the test lets it, as a real start
 * takes seconds; every other view is an empty snapshot.
 */
class FakeCore {
	static readonly OPEN = 1;
	static answerStart: (() => void) | null = null;
	readyState = 1;
	roster: string | null = null;
	onopen: (() => void) | null = null;
	onmessage: ((message: { data: string }) => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	constructor(readonly url: string) {
		setTimeout(() => this.onopen?.(), 0);
	}
	send(data: string) {
		const frame = JSON.parse(data) as Record<string, unknown>;
		const id = frame["id"] as string;
		const params = frame["params"] as Record<string, unknown> | undefined;
		const reply = (answer: Record<string, unknown>) => setTimeout(() => this.onmessage?.({ data: JSON.stringify(answer) }), 0);
		if ("sub" in frame) {
			if ((frame["sub"] as { view?: string }).view === "roster") this.roster = id;
			reply({ id, ok: true });
			reply({ sub: id, snapshot: [] });
		} else if (frame["cmd"] === "client.hello") {
			reply({ id, ok: true, result: { capabilities: ["threads", "threads2"] } });
		} else if (frame["cmd"] === "welcome") {
			reply({ id, ok: true, result: { providers: [], harnesses: [], defaultBackendId: "hotline", canRun: true, teammates: 0, setUp: false } });
		} else if (frame["cmd"] === "backends.list") {
			reply({ id, ok: true, result: [{ id: "hotline", name: "Hotline Agent" }] });
		} else if (frame["cmd"] === "persona.create") {
			const draft = params!["draft"] as { name: string };
			const made = entry("p1", draft.name);
			reply({ sub: this.roster, event: made });
			reply({ id, ok: true, result: made.persona });
		} else if (frame["cmd"] === "session.start") {
			FakeCore.answerStart = () => reply({ id, ok: true, result: null });
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
let dom: ReturnType<typeof installDom>["dom"];
let teardown: () => Promise<void>;

beforeEach(async () => {
	FakeCore.answerStart = null;
	({ dom, teardown } = installDom({ ResizeObserver: NoResize, WebSocket: FakeCore }));
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
	await teardown();
});

async function settle() {
	for (let turn = 0; turn < 8; turn++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 5));
		});
	}
}
const text = () => container.textContent ?? "";

test("the welcome's Add teammate ends on the new teammate's conversation, ready for hello", async () => {
	await act(async () => {
		root!.render(<App />);
	});
	await settle();
	expect(text()).toContain("Meet your first teammate");

	const name = container.querySelector<HTMLInputElement>('input[placeholder="Name your teammate"]')!;
	await act(async () => {
		Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, "value")!.set!.call(name, "Ada");
		name.dispatchEvent(new window.Event("input", { bubbles: true }));
	});
	const add = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Add teammate")!;
	await act(async () => {
		add.click();
	});
	await settle();

	// Open while the session is still starting, and still open once it has.
	expect(FakeCore.answerStart).not.toBeNull();
	opened();
	await act(async () => {
		FakeCore.answerStart!();
	});
	await settle();
	opened();
});

function opened() {
	expect(text()).not.toContain("Pick a teammate on the left.");
	expect(text()).not.toContain("Meet your first teammate");
	// The conversation is Ada's, its starters show, and the composer has the keys.
	expect(container.querySelector('[data-teammate-row][aria-current="true"]')?.getAttribute("aria-label")).toStartWith("Ada");
	expect(container.querySelector("fieldset[aria-label=\"Things to try\"]")).not.toBeNull();
	expect(document.activeElement?.tagName).toBe("TEXTAREA");
}
