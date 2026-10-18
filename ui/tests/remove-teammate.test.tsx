import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";

// Loading the window's modules reads the platform and the media queries it
// watches from `window`, before any test has a DOM.
const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });

// The shell's menu and its Remove dialog: the menu is kept to be pressed by
// the test, and the dialog says yes.
type TeammateMenu = { onDelete(): void };
let menu: TeammateMenu | null = null;
const native = await import("../src/native");
mock.module("../src/native", () => ({
	...native,
	confirmRemove: async () => true,
	popupTeammateMenu: async (actions: TeammateMenu) => {
		menu = actions;
	},
}));
const { App } = await import("../src/App");
const { replaceDesks } = await import("../src/desks");

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

const entry = (id: string) => ({
	persona: {
		id,
		name: id.toUpperCase(),
		goal: "",
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
 * A core whose roster holds teammates A and B. It answers `persona.delete`
 * only when the test lets it, so the window can be used while the removal
 * is on its way; every other view is an empty snapshot.
 */
class FakeCore {
	static readonly OPEN = 1;
	static all: FakeCore[] = [];
	static answerDelete: (() => void) | null = null;
	readyState = 1;
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
		const id = frame["id"];
		const reply = (answer: Record<string, unknown>) => setTimeout(() => this.onmessage?.({ data: JSON.stringify(answer) }), 0);
		if ("sub" in frame) {
			const roster = (frame["sub"] as { view?: string }).view === "roster";
			reply({ id, ok: true });
			reply({ sub: id, snapshot: roster ? [entry("a"), entry("b")] : [] });
		} else if (frame["cmd"] === "client.hello") {
			reply({ id, ok: true, result: { capabilities: ["threads", "threads2"] } });
		} else if (frame["cmd"] === "persona.delete") {
			FakeCore.answerDelete = () => reply({ id, ok: true, result: null });
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
	FakeCore.answerDelete = null;
	menu = null;
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

async function settle() {
	for (let turn = 0; turn < 5; turn++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 5));
		});
	}
}

const row = (name: string) => {
	const found = [...container.querySelectorAll<HTMLButtonElement>("[data-teammate-row]")].find((node) => node.getAttribute("aria-label")?.startsWith(name));
	if (found === undefined) throw new Error(`no rail row for ${name}`);
	return found;
};

test("a teammate picked while another's removal is on its way stays open", async () => {
	await act(async () => {
		root!.render(<App />);
	});
	await settle();
	await act(async () => {
		row("A").click();
	});
	expect(row("A").getAttribute("aria-current")).toBe("true");

	// Remove A from the rail's menu, and pick B before the core answers.
	await act(async () => {
		row("A").dispatchEvent(new dom.MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
	});
	await act(async () => {
		menu!.onDelete();
	});
	await settle();
	expect(FakeCore.answerDelete).not.toBeNull();
	await act(async () => {
		row("B").click();
	});
	expect(row("B").getAttribute("aria-current")).toBe("true");

	await act(async () => {
		FakeCore.answerDelete!();
	});
	await settle();
	expect(row("B").getAttribute("aria-current")).toBe("true");
});
