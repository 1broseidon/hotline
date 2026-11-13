import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Persona, RosterEntry, RunningSubagent, SessionInfo, ThreadSummary, TranscriptEvent } from "../src/generated/contract";
import type { ThreadRef } from "../src/components/Transcript";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

// task-399: work threads carry a kind label, and live subagents show their
// latest step in link rows, the composer's threads panel and thread list. The
// rail no longer nests them under their teammate (desk-threads, decision 3).

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Dock } = await import("../src/components/Dock");
const { Rail } = await import("../src/components/Rail");
const { liveThreads } = await import("../src/liveThreads");
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
		// The band says the kind and the head the title, as on every thread's pane.
		expect(container.querySelector(".band-kind")?.textContent).toBe("≡ Work thread");
		expect(container.querySelector(".pane-head .pane-title")?.textContent).toBe("Release notes for 0.49");
		// Still the conversation it was: its composer, End thread, and not a run's pane.
		expect(container.querySelector("textarea")).not.toBeNull();
		expect(container.querySelector('button[title="End this thread"]')).not.toBeNull();
		expect(container.querySelector(".run-pane")).toBeNull();
	});
});

describe("the rail", () => {
	// George, 2026-11-12 (desk-threads mockup, decision 3): a teammate's subagents are
	// no longer nested under it in the rail; the composer's threads count covers them.
	test("names each teammate once, with nothing nested under one whose subagents are at work", async () => {
		await act(async () => {
			root.render(
				<Rail
					entries={[entry(pork, [audit, models, crawl]), entry(lou)]}
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
				/>,
			);
		});
		const rows = [...container.querySelectorAll("[data-teammate-row]")].map((row) => row.getAttribute("aria-label"));
		expect(rows).toEqual(["Pork Chop", "Lou"]);
		expect(container.querySelector("[data-run], .rail-runs")).toBeNull();
		expect(container.textContent).not.toContain("Audit side-pane thread rows");
	});
});

describe("a subagent's row at the composer, and none in the conversation", () => {
	const link = (state: "live" | "closed"): TranscriptEvent => ({
		kind: "link", id: "link:run:r-audit", ts: 1, thread: "r-audit", threadKind: "run", personaId: "pork", title: "Audit side-pane thread rows", state,
		...(state === "closed" ? { end: "done" as const, elapsedMs: 214_000 } : {}),
	});
	const transcript = (event: TranscriptEvent) => <Transcript personaId="pork" name="Pork Chop" events={[event]} streaming={[]} live={false} focus={null} />;

	test("The conversation draws no row for a subagent, working or ended, and the composer's threads panel shows its latest step instead of only \"working\"", async () => {
		await act(async () => root.render(transcript(link("live"))));
		expect(container.querySelector(".link-row")).toBeNull();
		expect(container.textContent).not.toContain("Audit side-pane thread rows");
		await act(async () => root.render(transcript(link("closed"))));
		expect(container.querySelector(".link-row")).toBeNull();
		expect(container.textContent).not.toContain("Done in");

		// The composer's threads panel says each one's latest step beside its kind (task-437).
		const runs = [audit, models];
		const listedRuns = runs.map((run): ThreadSummary => ({ thread: { kind: "run", key: run.runId }, personaId: "pork", title: run.title, state: "live", startedAt: run.startedAt, updatedAt: run.startedAt, working: false, waiting: false }));
		const ada = entry(pork, runs);
		const rows = liveThreads({ listed: listedRuns, roster: [ada], entry: ada, jobs: [], nameOf: () => undefined, now: 10 });
		expect(rows.map((row) => row.line)).toEqual(["Helper · Fetch /v1/models · 200", "Helper · Read Dock.tsx"]);
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
		expect(container.querySelector('[data-thread="run:r-models"] .list-mark')?.getAttribute("title")).toBe("Helper");
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
