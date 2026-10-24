import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });
const { App } = await import("../src/App");
const { replaceDesks } = await import("../src/desks");

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

/** Ada, in the middle of a turn. */
const ada = {
	persona: {
		id: "ada",
		name: "Ada",
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
		personaId: "ada",
		state: "thinking",
		contextRestored: false,
		models: [],
		modes: [],
		configs: [],
		slashCommands: [],
		capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
	},
	waiting: false,
	drawing: false,
};

class FakeCore {
	static readonly OPEN = 1;
	static cancels: unknown[] = [];
	readyState = 1;
	onopen: (() => void) | null = null;
	onmessage: ((message: { data: string }) => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	constructor(readonly url: string) {
		setTimeout(() => this.onopen?.(), 0);
	}
	send(data: string) {
		const frame = JSON.parse(data) as Record<string, unknown>;
		const id = frame["id"];
		const reply = (answer: Record<string, unknown>) => setTimeout(() => this.onmessage?.({ data: JSON.stringify(answer) }), 0);
		if ("sub" in frame) {
			const roster = (frame["sub"] as { view?: string }).view === "roster";
			reply({ id, ok: true });
			reply({ sub: id, snapshot: roster ? [ada] : [] });
			return;
		}
		if (frame["cmd"] === "client.hello") {
			reply({ id, ok: true, result: { capabilities: ["threads", "threads2"] } });
		} else if (frame["cmd"] === "session.cancel") {
			FakeCore.cancels.push(frame["params"]);
			reply({ id, ok: true, result: null });
		} else {
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
let teardown: () => Promise<void>;

beforeEach(async () => {
	FakeCore.cancels = [];
	({ teardown } = installDom({
		ResizeObserver: NoResize,
		WebSocket: FakeCore,
		requestAnimationFrame: () => 0,
		cancelAnimationFrame: () => {},
	}));
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

test("the main conversation's Interrupt and Esc cut the turn as an interrupt, never the person's Stop", async () => {
	await act(async () => {
		root!.render(<App />);
	});
	await settle();
	const rail = container.querySelector<HTMLButtonElement>('[data-teammate-row][aria-label^="Ada"]')!;
	await act(async () => {
		rail.click();
	});
	await settle();
	const interrupt = container.querySelector<HTMLButtonElement>('[aria-label="Interrupt"]');
	if (interrupt === null) throw new Error(`No Interrupt in: ${container.textContent}`);
	await act(async () => interrupt.click());
	const field = container.querySelector("textarea");
	if (field === null) throw new Error(`No composer in: ${container.textContent}`);
	await act(async () => {
		field.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
	});
	await settle();
	expect(FakeCore.cancels).toEqual([
		{ personaId: "ada", interrupt: true },
		{ personaId: "ada", interrupt: true },
	]);
});
