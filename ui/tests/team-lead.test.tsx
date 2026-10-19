import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

// Loading the window's modules reads the platform and the media queries it
// watches from `window`, before any test has a DOM.
const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });

// The shell's teammate menu, kept to be read and pressed by the test.
type TeammateMenu = { lead: { lead: boolean; alone: boolean; onToggle(): void } };
let menu: TeammateMenu | null = null;
const native = await import("../src/native");
mock.module("../src/native", () => ({
	...native,
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

const entry = (id: string, createdAt: number, lead = false) => ({
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
		createdAt,
		updatedAt: createdAt,
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
	...(lead ? { lead: true } : {}),
});

type Entry = ReturnType<typeof entry>;

/**
 * A core whose roster is what the test says. The first snapshot is
 * `FakeCore.roster`; `FakeCore.push` sends rows and removals the way the
 * core does when the lead moves or a teammate goes. `persona.lead` is
 * recorded and answered, and changes nothing by itself: the roster's next
 * rows are the only thing the window may draw a lead from.
 */
class FakeCore {
	static readonly OPEN = 1;
	static roster: Entry[] = [];
	static leads: Array<Record<string, unknown>> = [];
	static live: FakeCore | null = null;
	readyState = 1;
	rosterSub: string | null = null;
	onopen: (() => void) | null = null;
	onmessage: ((message: { data: string }) => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	constructor(readonly url: string) {
		FakeCore.live = this;
		setTimeout(() => this.onopen?.(), 0);
	}
	reply(answer: Record<string, unknown>) {
		setTimeout(() => this.onmessage?.({ data: JSON.stringify(answer) }), 0);
	}
	send(data: string) {
		const frame = JSON.parse(data) as Record<string, unknown>;
		const id = frame["id"] as string;
		if ("sub" in frame) {
			const roster = (frame["sub"] as { view?: string }).view === "roster";
			if (roster) this.rosterSub = id;
			this.reply({ id, ok: true });
			this.reply({ sub: id, snapshot: roster ? FakeCore.roster : [] });
		} else if (frame["cmd"] === "client.hello") {
			this.reply({ id, ok: true, result: { capabilities: ["threads", "threads2"] } });
		} else if (frame["cmd"] === "welcome") {
			this.reply({ id, ok: true, result: { providers: [], harnesses: [], defaultBackendId: "hotline", canRun: true, teammates: 0, setUp: false } });
		} else if (frame["cmd"] === "persona.lead") {
			const params = frame["params"] as Record<string, unknown>;
			FakeCore.leads.push(params);
			this.reply({ id, ok: true, result: params["id"] ?? null });
		} else if ("cmd" in frame) {
			this.reply({ id, ok: false, error: "Not in this test." });
		}
	}
	static push(...frames: Array<Entry | { removed: string }>) {
		const core = FakeCore.live!;
		for (const frame of frames) {
			core.reply("removed" in frame ? { sub: core.rosterSub, removed: frame.removed } : { sub: core.rosterSub, event: frame });
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
	FakeCore.roster = [];
	FakeCore.leads = [];
	FakeCore.live = null;
	menu = null;
	({ teardown } = installDom({ ResizeObserver: NoResize, WebSocket: FakeCore }));
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
	for (let turn = 0; turn < 6; turn++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 5));
		});
	}
}

async function open(roster: Entry[]) {
	FakeCore.roster = roster;
	await act(async () => {
		root!.render(<App />);
	});
	await settle();
}

async function push(...frames: Array<Entry | { removed: string }>) {
	FakeCore.push(...frames);
	await settle();
}

const row = (name: string) => {
	const found = [...container.querySelectorAll<HTMLElement>("[data-teammate-row]")].find((node) => node.getAttribute("aria-label")?.startsWith(name));
	if (found === undefined) throw new Error(`no rail row for ${name}`);
	return found;
};
/** Whose rows wear the lead's star, by name. */
const badged = () =>
	[...container.querySelectorAll("[data-teammate-row]")]
		.filter((node) => node.querySelector('.rail-face-lead[title="Team lead"]') !== null)
		.map((node) => node.getAttribute("aria-label")?.split(",")[0]);
const prompt = () => container.querySelector('section[aria-label="Pick a team lead"]');
const button = (text: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find((one) => one.textContent === text);

async function rightClick(name: string) {
	await act(async () => {
		row(name).dispatchEvent(new window.MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
	});
}

test("the badge is on the roster's lead and follows it when the role moves", async () => {
	await open([entry("a", 1, true), entry("b", 2)]);
	expect(badged()).toEqual(["A"]);
	expect(row("A").getAttribute("aria-label")).toBe("A, team lead");

	await push(entry("a", 1), entry("b", 2, true));
	expect(badged()).toEqual(["B"]);
	expect(prompt()).toBeNull();
});

test("the menu sends the lead command and offers Remove as lead on the lead, leaving the roster to say who leads", async () => {
	await open([entry("a", 1, true), entry("b", 2), entry("c", 3)]);

	await rightClick("B");
	expect(menu!.lead.lead).toBe(false);
	await act(async () => menu!.lead.onToggle());
	await settle();
	expect(FakeCore.leads).toEqual([{ id: "b" }]);
	// Nothing moves until the core's rows say so.
	expect(badged()).toEqual(["A"]);

	await rightClick("A");
	expect(menu!.lead).toMatchObject({ lead: true, alone: false });
	await act(async () => menu!.lead.onToggle());
	await settle();
	expect(FakeCore.leads).toEqual([{ id: "b" }, {}]);
});

test("the only teammate leads, and its menu cannot remove it", async () => {
	await open([entry("a", 1, true)]);
	await rightClick("A");
	expect(menu!.lead).toMatchObject({ lead: true, alone: true });
	expect(prompt()).toBeNull();
});

// Not now lasts until the next launch, which a test file is one of, so each
// test that asks starts from a desk with a lead: a lead named ends a Not now.

test("two or more with no lead are asked to pick one, the oldest suggested, until Not now", async () => {
	await open([entry("b", 20, true), entry("a", 10), entry("c", 30)]);
	expect(prompt()).toBeNull();
	// The lead cleared, on another seat.
	await push(entry("b", 20));
	expect(prompt()?.textContent).toContain("A has been on the team longest");

	await act(async () => button("Make A lead")!.click());
	await settle();
	expect(FakeCore.leads).toEqual([{ id: "a" }]);

	await act(async () => button("Not now")!.click());
	expect(prompt()).toBeNull();
	// A teammate coming or going does not ask again.
	await push(entry("d", 40));
	expect(prompt()).toBeNull();
});

test("deleting the lead asks again while two or more remain, even after Not now", async () => {
	await open([entry("a", 1, true), entry("b", 2), entry("c", 3)]);
	await push(entry("a", 1));
	await act(async () => button("Not now")!.click());
	expect(prompt()).toBeNull();

	// A lead named ends the Not now, so losing it asks again.
	await push(entry("a", 1, true));
	await push({ removed: "a" });
	expect(prompt()?.textContent).toContain("B has been on the team longest");

	await push(entry("b", 2, true));
	expect(prompt()).toBeNull();
	await push({ removed: "b" });
	// One teammate left: the core makes it lead, and nobody is asked.
	expect(prompt()).toBeNull();
});

test("a desk of one asks nothing, and the welcome says the first teammate leads", async () => {
	await open([]);
	expect(container.textContent).toContain("Your first teammate leads the team.");
	expect(prompt()).toBeNull();
});
