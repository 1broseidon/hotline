import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Persona, RosterEntry, RunningSubagent, SessionInfo, ThreadSummary, TranscriptEvent } from "../src/generated/contract";
import type { ThreadRef } from "../src/components/Transcript";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

// task-399: work threads carry a kind label, and live subagents show their
// latest step in the rail, link rows, chip and thread list.

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Dock } = await import("../src/components/Dock");
const { Rail } = await import("../src/components/Rail");
const { SubagentChips } = await import("../src/components/Conversation");
const { Transcript } = await import("../src/components/Transcript");
const { replaceDesks, wireFor } = await import("../src/desks");
const { endedWords } = await import("../src/dock");
const { rememberFilter } = await import("../src/threadList");

const persona = (id: string, name: string): Persona => ({
	id, name, goal: "", backendId: "hotline", cwd: `/work/${id}`,
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
});
const session = (personaId: string): SessionInfo => ({
	personaId, state: "idle", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
	capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
});
const pork = persona("pork", "Pork Chop");
const lou = persona("lou", "Lou");
const entry = (one: Persona, subagents?: RunningSubagent[]): RosterEntry => ({
	persona: one, waiting: false, drawing: false, session: session(one.id), ...(subagents !== undefined ? { subagents } : {}),
});

const audit: RunningSubagent = { runId: "r-audit", title: "Audit side-pane thread rows", startedAt: 1, lastStep: "Read Dock.tsx", steps: 1 };
const models: RunningSubagent = { runId: "r-models", title: "Check ChatGPT model list", startedAt: 2, lastStep: "Fetch /v1/models · 200", steps: 3 };
const crawl: RunningSubagent = { runId: "r-crawl", title: "Crawl 12 sites", startedAt: 3, kind: "shell", command: "python3 crawl.py" };

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Handlers = { snapshot(items: unknown[]): void; event(item: unknown): void };
let listed: ThreadSummary[];
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
	listed = [];
	const connection = wireFor("local");
	const open = spyOn(connection, "onConnection").mockImplementation((watcher) => {
		watcher("open");
		return () => {};
	});
	restores.push(() => open.mockRestore());
	const subscription = spyOn(connection, "subscribe").mockImplementation(((_target: unknown, handlers: Handlers) => {
		queueMicrotask(() => handlers.snapshot([]));
		return () => {};
	}) as Wire["subscribe"]);
	restores.push(() => subscription.mockRestore());
	const command = spyOn(connection, "command").mockImplementation((async (cmd: string) => {
		if (cmd === "thread.list") return listed;
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

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

async function dock(open: ThreadRef | null, roster: RosterEntry[]) {
	await act(async () => {
		root.render(
			<Dock
				state={{ open }}
				onState={() => {}}
				onClose={() => {}}
				onOpenTeammate={() => {}}
				entry={roster[0] ?? null}
				roster={roster}
				width={360}
				onWidth={() => {}}
				overlay={false}
			/>,
		);
	});
	await settle();
}

const summary = (row: Partial<ThreadSummary> & Pick<ThreadSummary, "thread">): ThreadSummary => ({
	personaId: "pork", state: "live", startedAt: 1, updatedAt: 1, working: false, waiting: false, ...row,
});

describe("a work thread's pane", () => {
	test("A work thread's pane shows \"Work thread\"; nothing else about it changes", async () => {
		listed = [summary({ thread: { kind: "side", key: "s1" }, title: "Release notes for 0.49" })];
		await dock({ thread: { kind: "side", key: "s1" }, title: "Release notes for 0.49" }, [entry(pork)]);
		const head = container.querySelector(".pane-head");
		expect(head?.querySelector(".pane-kind")?.textContent).toBe("Work thread");
		expect(head?.querySelector(".pane-kind")?.hasAttribute("data-watching")).toBe(false);
		// Plain: no family line beside the label, and the band keeps the title.
		expect(head?.querySelector(".pane-family")).toBeNull();
		expect(head?.querySelector(".pane-title")).toBeNull();
		// Still the conversation it was: its composer, End thread, and not a run's pane.
		expect(container.querySelector("textarea")).not.toBeNull();
		expect(container.querySelector('button[title="End this thread"]')).not.toBeNull();
		expect(container.querySelector(".run-pane")).toBeNull();
	});
});

describe("the rail", () => {
	async function rail(entries: RosterEntry[], onOpenRun: (personaId: string, run: RunningSubagent) => void, openRun?: string) {
		await act(async () => {
			root.render(
				<Rail
					entries={entries}
					selectedId="pork"
					seen={{}}
					connection="open"
					onSelect={() => {}}
					onEdit={() => {}}
					onDelete={() => {}}
					onPin={() => {}}
					onLead={() => {}}
					onAddDesk={() => {}}
					onNew={() => {}}
					settings={null}
					search={null}
					deskKey={null}
					onOpenRun={onOpenRun}
					openRun={openRun}
				/>,
			);
		});
	}

	test("The rail nests a teammate's live subagents under it, each with its title and latest step; it leaves the rail when it reports", async () => {
		const opened: [string, string][] = [];
		await rail([entry(pork, [audit, models, crawl]), entry(lou)], (personaId, run) => opened.push([personaId, run.runId]), "r-audit");
		const rows = [...container.querySelectorAll("[data-teammate-row], .rail-run")];
		// Under Pork Chop, before Lou; a command left running is not a subagent.
		expect(rows.map((row) => row.getAttribute("data-run") ?? row.getAttribute("aria-label"))).toEqual(["Pork Chop", "r-audit", "r-models", "Lou"]);
		const first = container.querySelector<HTMLButtonElement>('[data-run="r-audit"]')!;
		expect(first.querySelector(".rail-run-title")?.textContent).toBe("Audit side-pane thread rows");
		expect(first.querySelector(".rail-run-step")?.textContent).toBe("Read Dock.tsx");
		expect(first.getAttribute("aria-current")).toBe("true");
		expect(container.querySelector('[data-run="r-models"] .rail-run-step')?.textContent).toBe("Fetch /v1/models · 200");
		await act(async () => first.click());
		expect(opened).toEqual([["pork", "r-audit"]]);

		// Its latest step moves with the roster row.
		await rail([entry(pork, [{ ...audit, lastStep: "Search \"subagent\" in ui/src" }, models]), entry(lou)], () => {});
		expect(container.querySelector('[data-run="r-audit"] .rail-run-step')?.textContent).toBe("Search \"subagent\" in ui/src");

		// Reported, it is off the roster row, and off the rail.
		await rail([entry(pork, [models]), entry(lou)], () => {});
		expect(container.querySelector('[data-run="r-audit"]')).toBeNull();
		await rail([entry(pork), entry(lou)], () => {});
		expect(container.querySelector(".rail-runs")).toBeNull();
	});
});

describe("a subagent's line and chip", () => {
	const link = (state: "live" | "closed"): TranscriptEvent => ({
		kind: "link", id: "link:run:r-audit", ts: 1, thread: "r-audit", threadKind: "run", personaId: "pork", title: "Audit side-pane thread rows", state,
		...(state === "closed" ? { end: "done" as const, elapsedMs: 214_000 } : {}),
	});
	const transcript = (event: TranscriptEvent, subagents?: RunningSubagent[]) => (
		<Transcript personaId="pork" name="Pork Chop" events={[event]} streaming={[]} live={false} focus={null} subagents={subagents} />
	);

	test("A subagent's link row and the composer chip show its latest step instead of only \"working\"", async () => {
		await act(async () => root.render(transcript(link("live"), [audit])));
		const row = container.querySelector(".link-row");
		expect(row?.querySelector(".link-row-outcome")?.textContent).toBe("· Read Dock.tsx");
		expect(row?.getAttribute("aria-label")).toBe("Subagent · Audit side-pane thread rows · Read Dock.tsx. Open it");
		// Before its first step it works, and once ended it says how it went.
		await act(async () => root.render(transcript(link("live"), [{ runId: audit.runId, title: audit.title, startedAt: audit.startedAt }])));
		expect(container.querySelector(".link-row-outcome")?.textContent).toBe("· Working");
		await act(async () => root.render(transcript(link("closed"))));
		expect(container.querySelector(".link-row-outcome")?.textContent).toBe("· Done in 3 min 34 s");

		// One chip names what the subagent did last.
		await act(async () => root.render(<SubagentChips subagents={[audit]} opened={() => false} onOpenThread={() => {}} />));
		const chip = container.querySelector<HTMLButtonElement>("button");
		expect(chip?.textContent).toBe("Subagent · Read Dock.tsx");
		expect(chip?.getAttribute("aria-label")).toBe("Subagent working: Audit side-pane thread rows, Read Dock.tsx");

		// Several are counted, and the menu says each one's latest step beside its title.
		await act(async () => root.render(<SubagentChips subagents={[audit, models]} opened={() => false} onOpenThread={() => {}} />));
		await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
		const items = [...document.querySelectorAll('[role="menuitem"]')];
		expect(items.map((item) => item.querySelector(".menu-item-detail")?.textContent)).toEqual(["Read Dock.tsx", "Fetch /v1/models · 200"]);
	});
});

describe("the thread list", () => {
	test("The thread list shows a working subagent's latest step with the beat, and an ended one's result", async () => {
		listed = [
			summary({ thread: { kind: "run", key: "r-models" }, title: "Check ChatGPT model list", working: true }),
			summary({ thread: { kind: "side", key: "s1" }, title: "Release notes for 0.49", preview: "Yes, and keep it short." }),
			summary({ thread: { kind: "run", key: "r-audit" }, title: "Audit side-pane thread rows", working: true }),
			summary({ thread: { kind: "run", key: "r-map" }, title: "Map browser stream latency", state: "closed", end: "done", startedAt: 0, updatedAt: 221_000, preview: "Input to picture is about 26 ms." }),
		];
		// All: the ended one is long ago, which Active folds away.
		rememberFilter("pork", "all");
		await dock(null, [entry(pork, [audit, models])]);
		const line = (id: string) => container.querySelector(`[data-thread="${id}"] .list-line`)?.textContent;
		const beats = (id: string) => container.querySelector(`[data-thread="${id}"] .dock-dot.beat`) !== null;

		// A working one: its kind, and its latest step, beating.
		expect(container.querySelector('[data-thread="run:r-models"] .list-mark')?.getAttribute("title")).toBe("Subagent");
		expect(line("run:r-models")).toBe("Fetch /v1/models · 200");
		expect(line("run:r-audit")).toBe("Read Dock.tsx");
		expect(beats("run:r-models")).toBe(true);

		// An ended one: what it reported, and how long it took.
		expect(line("run:r-map")).toBe("Reported · Input to picture is about 26 ms. · 3:41");
		expect(beats("run:r-map")).toBe(false);
	});

	test("a command left running is a command, not a subagent; a stopped subagent says so, and a finished command says Done", async () => {
		listed = [summary({ thread: { kind: "run", key: "r-crawl" }, title: "Crawl 12 sites", working: true })];
		await dock(null, [entry(pork, [crawl])]);
		expect(container.querySelector('[data-thread="run:r-crawl"] .list-mark')?.getAttribute("title")).toBe("Command");
		expect(container.querySelector('[data-thread="run:r-crawl"] .list-line')?.textContent).toBe("python3 crawl.py");
		expect(endedWords(summary({ thread: { kind: "run", key: "x" }, state: "closed", end: "cancelled", updatedAt: 7_000, startedAt: 1_000 }))).toBe("Stopped · 0:06");
		expect(endedWords(summary({ thread: { kind: "run", key: "x" }, state: "closed", end: "done", updatedAt: 7_000, startedAt: 1_000 }), true)).toBe("Done · 0:06");
	});
});
