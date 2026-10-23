import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Window } from "happy-dom";
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

const ada = {
	persona: {
		id: "ada",
		name: "Ada",
		goal: "",
		backendId: "claude-acp",
		accountId: "gone",
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
};

const failure = {
	kind: "account_unavailable",
	code: "unknown_account",
	accountId: "gone",
	title: "Ada can't start",
	summary: "Pick a Claude Code account for Ada: the one it was set to is not on this desk.",
	details: "account gone is not listed",
	phase: "start",
};
const refusal = `${failure.title}: ${failure.summary}\n\n${JSON.stringify({ hotlineFailure: failure })}`;

class FakeCore {
	static readonly OPEN = 1;
	static commands: string[] = [];
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
		FakeCore.commands.push(String(frame["cmd"]));
		if (frame["cmd"] === "client.hello") {
			reply({ id, ok: true, result: { capabilities: ["threads", "threads2"] } });
		} else if (frame["cmd"] === "session.start") {
			reply({ id, ok: false, error: refusal });
		} else if (frame["cmd"] === "accounts.list") {
			reply({ id, ok: true, result: { agents: [{ backendId: "claude-acp", name: "Claude Code", supportsAccounts: true, takesEmail: true }], accounts: [], credentials: [] } });
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
let dom: Window;
let teardown: () => Promise<void>;

beforeEach(async () => {
	FakeCore.commands = [];
	({ dom, teardown } = installDom({
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
const text = () => container.textContent ?? "";
function button(label: string): HTMLButtonElement {
	const found = [...container.querySelectorAll<HTMLButtonElement>("button")].find((node) => node.textContent === label || node.getAttribute("aria-label") === label);
	if (!found) throw new Error(`Missing button ${label}: ${text()}`);
	return found;
}

test("a start refused for the teammate's account says so as a card with Choose account", async () => {
	await act(async () => {
		root!.render(<App />);
	});
	await settle();
	const rail = container.querySelector<HTMLButtonElement>('[data-teammate-row][aria-label^="Ada"]')!;
	await act(async () => {
		rail.click();
	});
	await settle();

	const field = container.querySelector<HTMLTextAreaElement>("section.conversation textarea")!;
	await act(async () => {
		Object.getOwnPropertyDescriptor(dom.HTMLTextAreaElement.prototype, "value")!.set!.call(field, "hello");
		field.dispatchEvent(new window.Event("input", { bubbles: true }));
	});
	await act(async () => {
		button("Send").click();
	});
	await settle();
	expect(FakeCore.commands).toContain("session.start");

	const band = container.querySelector<HTMLElement>("section.conversation")!;
	expect(band.textContent).toContain("Pick a Claude Code account");
	expect(band.textContent).toContain("Pick a Claude Code account for Ada: the one it was set to is not on this desk.");
	expect(band.textContent).not.toContain("hotlineFailure");
	expect(container.querySelector('aside[aria-label="Ada\'s settings"]')).toBeNull();

	await act(async () => {
		button("Choose account").click();
	});
	await settle();
	const panel = container.querySelector<HTMLElement>('aside[aria-label="Ada\'s settings"]');
	expect(panel).not.toBeNull();
	expect(panel!.textContent).toContain("Claude Code account");
});
