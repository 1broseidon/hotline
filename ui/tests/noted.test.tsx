import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Aside, LinkState, Persona, RosterEntry, SessionInfo, SessionState, ThreadSummary, TranscriptEvent } from "../src/generated/contract";
import type { ThreadRef } from "../src/components/Transcript";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Dock } = await import("../src/components/Dock");
const { replaceDesks, wireFor } = await import("../src/desks");
const { rememberFilter } = await import("../src/threadList");

const persona = (id: string, name: string): Persona => ({
	id, name, goal: "", backendId: "hotline", cwd: `/work/${id}`,
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
});
const goldie = persona("goldie", "Goldie");
const quill = persona("quill", "Quill");
const huck = persona("huck", "Huck");
const session = (personaId: string, state: SessionState = "idle"): SessionInfo => ({
	personaId, state, contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
	capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
});
const entry = (one: Persona, extra: Partial<RosterEntry> = {}): RosterEntry => ({ persona: one, waiting: false, drawing: false, session: session(one.id), ...extra });

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

const release = { kind: "side" as const, key: "release" };
const aside = (id: string, ts: number, extra: Partial<Aside> = {}): Aside => ({
	id, ts, to: "quill", from: { thread: release }, by: "goldie", text: `aside ${id}`, level: "note", ...extra,
});

/** What the desk answers, by command; and what the window sent it. */
let answers: Record<string, (params: unknown) => unknown>;
let sent: { cmd: string; params: unknown }[];
/** The room's lines: the person's name, when a test gives one. */
let roomLines: unknown[];
/** The open thread's lines, and its state on the list. */
let threadLines: TranscriptEvent[];
let threadState: LinkState;
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;

beforeEach(async () => {
	restores = [];
	const installed = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	Object.assign(globalThis, { HTMLElement: installed.dom.HTMLElement });
	restores.push(() => Reflect.deleteProperty(globalThis, "HTMLElement"));
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	answers = {};
	sent = [];
	roomLines = [];
	threadLines = [];
	threadState = "live";
	const connection = wireFor("local");
	const open = spyOn(connection, "onConnection").mockImplementation((watcher) => {
		watcher("open");
		return () => {};
	});
	restores.push(() => open.mockRestore());
	const subscription = spyOn(connection, "subscribe").mockImplementation(((target: "room" | { threadId?: { kind: string; key: string } }, handlers: { snapshot(items: unknown[]): void }) => {
		const first = target === "room" ? roomLines : target.threadId?.key === release.key ? threadLines : [];
		queueMicrotask(() => handlers.snapshot(first));
		return () => {};
	}) as Wire["subscribe"]);
	restores.push(() => subscription.mockRestore());
	const command = spyOn(connection, "command").mockImplementation((async (cmd: string, params: unknown) => {
		sent.push({ cmd, params });
		const answer = answers[cmd];
		if (answer !== undefined) return answer(params);
		if (cmd === "thread.list") return listed();
		throw new Error(`Not in this test: ${cmd}`);
	}) as Wire["command"]);
	restores.push(() => command.mockRestore());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	replaceDesks([]);
	for (const restore of restores.reverse()) restore();
	await teardown();
});

/** Goldie's work thread, handed over by Huck when `handed`. */
let handed = false;
function listed(): ThreadSummary[] {
	return [{
		thread: release,
		personaId: "goldie",
		title: "release 0.11.1",
		state: threadState,
		...(handed ? { opener: { personaId: "huck", name: "Huck" } } : {}),
		startedAt: 1,
		updatedAt: 1,
		working: false,
		waiting: false,
	}];
}

const settle = () =>
	act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});

describe("an aside in a conversation", () => {
	async function transcript(personaId: string, here: { kind: "dm" | "side"; key: string }, events: TranscriptEvent[], opened: ThreadRef[] = []) {
		const { Transcript } = await import("../src/components/Transcript");
		const people = new Map([["goldie", { name: "Goldie" }], ["quill", { name: "Quill" }], ["huck", { name: "Huck" }]]);
		await act(async () => {
			root.render(
				<Transcript
					personaId={personaId}
					{...(here.kind === "side" ? { thread: here } : {})}
					name="Quill"
					people={people}
					events={events}
					streaming={[]}
					live={false}
					focus={null}
					onOpenThread={(open) => opened.push(open)}
				/>,
			);
		});
	}

	test("is a quiet (·) line in the receiver's main conversation, never a bubble, and opens its thread", async () => {
		const opened: ThreadRef[] = [];
		await transcript("quill", { kind: "dm", key: "quill" }, [
			{ kind: "user", id: "u1", ts: 1, text: "Three names for the share button." },
			{ kind: "aside", ...aside("x", 2, { text: "Answered Pork Chop for you: the Q3 numbers are in." }) },
		], opened);
		const line = container.querySelector<HTMLButtonElement>("button.aside-line");
		expect(line).not.toBeNull();
		expect(line!.textContent).toContain("(·)");
		expect(line!.textContent).toContain("from Goldie · work thread");
		expect(line!.textContent).toContain("Answered Pork Chop for you: the Q3 numbers are in.");
		expect(line!.querySelector(".instrument")).not.toBeNull();
		expect(line!.closest(".speech")).toBeNull();
		expect(container.querySelectorAll(".speech").length).toBe(1);
		await act(async () => line!.click());
		expect(opened).toEqual([{ thread: release }]);
	});

	const loopedIn = () => ({ kind: "aside" as const, ...aside("l", 2, { level: "nudge", loopedIn: true, text: "George looped you in." }) });

	test("of a Loop in is from the person, by their name", async () => {
		roomLines = [{ kind: "setting", id: "personName", value: "George" }];
		await transcript("quill", { kind: "dm", key: "quill" }, [loopedIn()]);
		await settle();
		const line = container.querySelector(".aside-line")?.textContent ?? "";
		expect(line).toContain("from George · work thread · nudge");
		expect(line).not.toContain("Goldie");
	});

	test("of a Loop in is from you while the person has given no name", async () => {
		await transcript("quill", { kind: "dm", key: "quill" }, [loopedIn()]);
		await settle();
		expect(container.querySelector(".aside-line")?.textContent).toContain("from you · work thread · nudge");
	});

	test("on the thread that left it says whom it went to and at what level, and opens nothing", async () => {
		await transcript("goldie", release, [{ kind: "aside", ...aside("x", 2, { level: "nudge", text: "Build 46 is up." }) }]);
		const line = container.querySelector(".aside-line");
		expect(line?.tagName).toBe("P");
		expect(line?.textContent).toContain("to Quill · nudge");
		expect(line?.textContent).toContain("Build 46 is up.");
	});

	test("of a Loop in says who was looped in, and the one answer is said whole under their face", async () => {
		await transcript("goldie", release, [
			{ kind: "aside", ...aside("x", 2, { level: "nudge", loopedIn: true, text: "The person looped you in." }) },
			{
				kind: "delivery", id: "exchange-result:r1", ts: 3, text: "It's \"Send to your computer\".\n\nFixed in the strings this morning.",
				from: { kind: "pair", thread: "goldie~quill", request: "r1" },
				cause: { kind: "peer", requestId: "r1", personaId: "quill", name: "Quill", threadKey: "goldie~quill", status: "done", about: "looped in" },
			},
		]);
		expect(container.querySelector(".aside-line")?.textContent).toMatch(/^Quill was looped in nudge · /);
		const said = container.querySelector(".said-group");
		expect(said?.querySelector(".said-name")?.textContent).toContain("Quill");
		expect(said?.querySelector(".speech")?.textContent).toContain("Fixed in the strings this morning.");
		expect(container.querySelector(".hung-line")).toBeNull();
	});
});

describe("Threads in the teammate's menu", () => {
	test("stands between Threads and Remove with how many are new", async () => {
		const { teammateActions } = await import("../src/components/Conversation");
		const { ModelPicker } = await import("../src/components/ModelPicker");
		const done: string[] = [];
		const actions = (unseen: number) =>
			teammateActions({
				inspectorOpen: false,
				chapterBusy: false,
				unseen,
				onToggleInspector: () => {},
				onStartChapter: () => {},
				onOpenThreadList: () => done.push("threads"),
				onDelete: () => {},
			});
		const render = (unseen: number) =>
			act(async () => {
				root.render(
					<ModelPicker
						choices={[{ id: "m", name: "Sonnet" }]}
						modelId="m"
						effort={null}
						label="Model"
						onModel={() => {}}
						onEffort={() => {}}
						teammate={{ name: "Quill", goal: "", working: false, face: () => <span />, actions: actions(unseen) }}
					/>,
				);
			});
		await render(3);
		await act(async () => container.querySelector<HTMLButtonElement>("button.model-pill")!.click());
		const items = () => [...document.querySelectorAll<HTMLElement>('[role="menu"] .menu-item .menu-item-text')].map((one) => one.textContent);
		// Asides are rows of the Threads list: its menu item counts the new ones, and there is no second item.
		expect(items().slice(-4)).toEqual(["Settings", "New chapter", "Threads3 new", "Remove"]);
		const threads = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"] .menu-item')].find((one) => one.textContent?.startsWith("Threads"))!;
		await act(async () => threads.click());
		expect(done).toEqual(["threads"]);
		await render(0);
		await act(async () => container.querySelector<HTMLButtonElement>("button.model-pill")!.click());
		expect(items()).toContain("Threads");
		expect(items()).not.toContain("Asides");
	});
});

describe("the notes in the Threads list", () => {
	async function showNotes(roster: RosterEntry[], states: unknown[] = []) {
		rememberFilter(roster[1]!.persona.id, "teammates");
		await act(async () => {
			root.render(
				<Dock
					state={{ open: null }}
					onState={(next) => states.push(next)}
					onClose={() => {}}
					onOpenTeammate={() => {}}
					entry={roster[1]!}
					roster={roster}
					width={360}
					onWidth={() => {}}
					overlay={false}
				/>,
			);
		});
		await settle();
		await settle();
	}
	const notes = () => [...container.querySelectorAll<HTMLElement>('.list-row[data-kind="note"]')];

	test("lists each with its words, who and where it came from, newest first, marks them read, and opens a thread", async () => {
		answers["aside.list"] = () => [
			aside("e1", 10, { seen: 15, text: "All 297 tests passed." }),
			aside("n2", 30, { level: "person", text: "The share button still has two names. Which one?", by: "huck" }),
			aside("n1", 20, { level: "nudge", text: "Build 46 is up." }),
		];
		answers["aside.seen"] = () => null;
		const states: unknown[] = [];
		await showNotes([entry(goldie), entry(quill, { asides: 2, asidesNew: 2 }), entry(huck)], states);
		const rows = notes();
		expect(rows.map((row) => row.querySelector(".list-title-text")?.textContent)).toEqual([
			"The share button still has two names. Which one?",
			"Build 46 is up.",
			"All 297 tests passed.",
		]);
		expect(rows[0]!.querySelector(".list-line")?.textContent).toBe("from Huck · release 0.11.1 · to you ›");
		expect(rows[0]!.getAttribute("data-level")).toBe("person");
		expect(rows[1]!.querySelector(".list-line")?.textContent).toBe("from Goldie · release 0.11.1 ›");
		expect(sent.some((one) => one.cmd === "aside.seen" && (one.params as { personaId: string }).personaId === "quill")).toBe(true);

		await act(async () => rows[1]!.click());
		expect(states).toEqual([{ open: { thread: release, title: "release 0.11.1" } }]);
	});

	test("lists a Loop in as the person's, by their name rather than the thread's teammate", async () => {
		roomLines = [{ kind: "setting", id: "personName", value: "George" }];
		answers["aside.list"] = () => [aside("l1", 40, { level: "nudge", loopedIn: true, text: "George looped you in." })];
		answers["aside.seen"] = () => null;
		await showNotes([entry(goldie), entry(quill, { asides: 1, asidesNew: 1 })]);
		const line = notes()[0]?.querySelector(".list-line")?.textContent;
		expect(line).toBe("from George · release 0.11.1 ›");
	});

	test("names an inbound aside's source, with no thread to open", async () => {
		const { by: _by, ...fromOutside } = aside("o1", 40, {
			from: { inbound: "gh" },
			sourceName: "hotline repo",
			text: "CI failed on main.",
		});
		answers["aside.list"] = () => [fromOutside];
		answers["aside.seen"] = () => null;
		await showNotes([entry(goldie), entry(quill, { asides: 1, asidesNew: 1 })]);
		const row = notes()[0];
		expect(row?.tagName).toBe("DIV");
		expect(row?.querySelector(".list-line")?.textContent).toBe("from hotline repo");
		expect(row?.querySelector(".list-title-text")?.textContent).toBe("CI failed on main.");
	});
});

describe("a work thread's header", () => {
	async function showThread(roster: RosterEntry[]) {
		await act(async () => {
			root.render(
				<Dock
					state={{ open: { thread: release, title: "release 0.11.1" } }}
					onState={() => {}}
					onClose={() => {}}
					onOpenTeammate={() => {}}
					entry={roster[0]!}
					roster={roster}
					width={360}
					onWidth={() => {}}
					overlay={false}
				/>,
			);
		});
		await settle();
	}

	test("loops in a colleague, said with what each is doing", async () => {
		handed = false;
		answers["thread.loop_in"] = () => null;
		await showThread([entry(goldie), entry(quill), entry(huck, { session: session("huck", "thinking") })]);
		const loopIn = container.querySelector<HTMLButtonElement>('[aria-label="Loop in a colleague"]');
		expect(loopIn?.textContent).toBe("Loop in…");
		await act(async () => loopIn!.click());
		const items = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"] .menu-item')];
		expect(items.map((one) => one.querySelector(".menu-item-text")?.textContent)).toEqual(["QuillIdle · a nudge wakes it", "HuckWorking · a thread answers"]);
		await act(async () => items[0]!.click());
		await settle();
		expect(sent.find((one) => one.cmd === "thread.loop_in")?.params).toEqual({ thread: release, personaId: "quill" });
	});

	test("ends a thread a teammate handed over with End thread, where Archive was", async () => {
		handed = true;
		answers["thread.close"] = () => null;
		await showThread([entry(goldie), entry(quill), entry(huck)]);
		expect(container.textContent).toContain("Huck handed this to Goldie.");
		expect(container.querySelector('button[title="Archive this thread"]')).toBeNull();
		const end = container.querySelector<HTMLButtonElement>('button[title="End this thread"]');
		expect(end?.textContent).toBe("End thread");
		await act(async () => end!.click());
		await settle();
		expect(sent.find((one) => one.cmd === "thread.close")?.params).toEqual({ thread: release });
	});

	test("has neither once the thread has ended: Continue brings it back", async () => {
		handed = false;
		threadState = "closed";
		await showThread([entry(goldie), entry(quill)]);
		expect(container.querySelector('[aria-label="Loop in a colleague"]')).toBeNull();
		expect(container.querySelector('button[title="End this thread"]')).toBeNull();
		expect(container.querySelector('button[title="Resume this thread"]')).not.toBeNull();
	});
});
