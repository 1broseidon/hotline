import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { RunningSubagent, ThreadSummary, TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

/*
 * The handoff, pair and command panes of epic-37 (docs/design/thread-panes.html),
 * each test named for the Done-when line of task-394, task-395 or task-396 it proves.
 */

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { HandoffPane } = await import("../src/components/thread/HandoffPane");
const { PairPane, pairLines } = await import("../src/components/thread/PairPane");
const { CommandPane } = await import("../src/components/thread/CommandPane");
const { LinkRow } = await import("../src/components/transcript/LinkRow");
const { liveThreads } = await import("../src/liveThreads");
const { Transcript } = await import("../src/components/Transcript");
const { wire } = await import("../src/wire");

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let restores: (() => void)[];
/** What the panes asked the desk, in order. */
let sent: { cmd: string; params: unknown }[];
/** What `process.output` answers. */
let output: { command: string; tail: string } | null;

beforeEach(async () => {
	restores = [];
	sent = [];
	output = null;
	const installed = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	measured = { scrollHeight: 0, clientHeight: 0 };
	const proto = installed.dom.HTMLElement.prototype;
	Object.defineProperty(proto, "scrollHeight", { configurable: true, get: () => measured.scrollHeight });
	Object.defineProperty(proto, "clientHeight", { configurable: true, get: () => measured.clientHeight });
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		sent.push({ cmd, params });
		if (cmd === "process.output") {
			if (output === null) throw new Error("That process is not running.");
			return output;
		}
		return null;
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	for (const restore of restores.reverse()) restore();
	await teardown();
});

async function render(node: React.ReactNode) {
	await act(async () => root.render(node));
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

/** What every element measures, so a test can say whether words overflow their six lines. */
let measured: { scrollHeight: number; clientHeight: number };
const text = (selector: string) => container.querySelector(selector)?.textContent;
const all = (selector: string) => [...container.querySelectorAll(selector)].map((one) => one.textContent);

/* ------------------------------------------------------------- handoff */

describe("task-394 a handoff's pane", () => {
	const thread = { kind: "side" as const, key: "w1" };
	const request: TranscriptEvent = {
		kind: "delivery", id: "brief", ts: 1, text: "hotline.dev's What's new still lists the old sign-in. Check every 0.49 claim against CHANGELOG.md.",
		cause: { kind: "handoff", requestId: "r1", personaId: "elvis", name: "Elvis", threadKey: "elvis~poe", about: "hotline.dev's What's new" },
	};
	const read: TranscriptEvent = { kind: "tool", id: "tool:c1", ts: 2, toolCallId: "c1", title: "Read CHANGELOG.md", toolKind: "read", status: "completed" };
	const reply: TranscriptEvent = { kind: "agent", id: "a1", ts: 3, text: "Two lines are stale." };
	const stepIn: TranscriptEvent = { kind: "user", id: "u1", ts: 4, text: "Check the phone page too." };
	const pane = (events: TranscriptEvent[], live: boolean, streaming: { messageId: string; kind: "agent" | "thought"; text: string; ts: number }[] = []) => (
		<HandoffPane thread={thread} events={events} streaming={streaming} live={live} status={live ? "live" : "stopped"} receiver={{ id: "poe", name: "Poe" }} sender={{ personaId: "elvis", name: "Elvis" }} people={new Map()} />
	);

	test("shows the request as words on a rule, clamped, with who it is from at the rule's end", async () => {
		await render(pane([request, read, reply], false));
		expect(container.querySelector(".pane-task")?.getAttribute("aria-label")).toBe("Request");
		expect(text(".pane-task .spine-clamp")).toBe(request.kind === "delivery" ? request.text : "");
		expect(text(".spine-rule .pane-task-from")).toEndWith("Elvis");
		// The request is the card, not a line of the stream as well.
		expect(all(".speech").some((one) => one?.includes("old sign-in"))).toBe(false);
	});

	test("shows the receiver's steps and words live, as in the subagent pane", async () => {
		// Words land a whole bubble at a time, as they do in the conversation.
		await render(pane([request, read], true, [{ messageId: "m2", kind: "agent", text: "Sending Elvis the lines.\n\nThe first", ts: 5 }]));
		expect(container.querySelector(".pane-stream .trail")).not.toBeNull();
		expect(text(".pane-stream .steps-heading")).toBe("Steps1");
		// Activity carries the page's one status mark.
		expect(container.querySelector(".spine-rule[data-status]")?.textContent).toBe("Activity");
		expect(container.querySelector(".spine-rule[data-status]")?.getAttribute("data-status")).toBe("live");
		expect(all(".pane-stream .speech").some((one) => one?.includes("Sending Elvis the lines."))).toBe(true);
		// After its words, a turn still running says it is at work; its open steps say so themselves.
		await render(pane([request, read, reply], true));
		expect(text(".pane-thinking")).toBe("Thinking");
		await render(pane([request, read, reply], false));
		expect(container.querySelector(".pane-thinking")).toBeNull();
	});

	test('labels its replies to the sender "<receiver> → <sender>", and the person\'s lines as theirs', async () => {
		await render(pane([request, read, reply, stepIn], false));
		const name = (said: string) => [...container.querySelectorAll(".speech")].find((one) => one.textContent?.includes(said))?.parentElement?.querySelector(".said-name")?.textContent;
		expect(name("Two lines are stale.")).toBe("Poe → Elvis");
		expect(name("Check the phone page too.")).toBe("You");
		expect([...container.querySelectorAll(".said-me")].map((one) => one.textContent)).toEqual(["Check the phone page too."]);
	});
});

/* ---------------------------------------------------------------- pair */

describe("task-395 a pair's pane", () => {
	const poe = { id: "poe", name: "Poe" };
	const lou = { id: "linked:grizzly:lou", name: "Lou on grizzly" };
	const key = `${poe.id}~${lou.id}`;
	const events: TranscriptEvent[] = [
		{ kind: "user", id: "p1", ts: 1, text: "What does the phone say on a sign-in card?" },
		{ kind: "agent", id: "l1", ts: 2, text: "Open Browser and Visit anyway." },
	];
	const row = (state: ThreadSummary["state"]): ThreadSummary => ({
		thread: { kind: "pair", key }, personaId: poe.id, withPersonaId: lou.id, withName: lou.name,
		state, startedAt: 1, updatedAt: 2, working: state === "live", waiting: false,
	});
	const pane = (state: ThreadSummary["state"]) => (
		<PairPane threadKey={key} self={poe} other={lou} row={row(state)} events={events} people={new Map()} onBack={() => {}} onClose={() => {}} />
	);

	test('says Pair alone in its band, "read only" in its state line, titled "Conversation with <B>" with a linked-desk teammate by its desk name', async () => {
		await render(pane("parked"));
		expect(text(".band-kind")).toBe("Pair");
		expect(text(".spine-state")).toStartWith("Earlier· read only");
		expect(text(".spine-title")).toBe("Conversation with Lou on grizzly");
		expect(container.querySelectorAll(".spine-facts > div").length).toBeLessThanOrEqual(3);
	});

	test("draws each line on the one text edge, in a well, under its teammate's name with their small face in the gutter", async () => {
		await render(pane("live"));
		const lines = [...container.querySelectorAll(".pair-line")];
		expect(lines.map((line) => line.querySelector(".said-name")?.textContent)).toEqual(["Poe", "Lou on grizzly"]);
		expect(lines.map((line) => line.querySelector(".pair-line-said > .spine-clamp") !== null)).toEqual([true, true]);
		expect(lines.map((line) => line.querySelector(".avatar")?.textContent)).toEqual(["P", "L"]);
		expect(lines.map((line) => line.querySelector(".pair-line-said .markdown, .pair-line-said p:last-child")?.textContent)).toEqual([
			"What does the phone say on a sign-in card?",
			"Open Browser and Visit anyway.",
		]);
		// The other teammate's first in the key writes as `user`: the sides follow the key, not the event's kind.
		expect(pairLines(`${lou.id}~${poe.id}`, events, poe, lou).map((line) => (line.kind === "said" ? line.who.name : ""))).toEqual(["Lou on grizzly", "Poe"]);
	});

	test("a long line of the old conversation is clamped to six lines in its well, Show all opening it in place", async () => {
		measured = { scrollHeight: 400, clientHeight: 132 };
		await render(pane("closed"));
		const first = container.querySelector(".pair-line-said")!;
		expect(first.querySelector(".spine-clamp")?.hasAttribute("data-clipped")).toBe(true);
		const more = first.querySelector<HTMLButtonElement>(".spine-more")!;
		expect(more.textContent).toBe("Show all");
		await act(async () => more.click());
		expect(first.querySelector(".spine-clamp")?.hasAttribute("data-open")).toBe(true);
		expect(more.textContent).toBe("Show less");
		measured = { scrollHeight: 0, clientHeight: 0 };
	});

	test("a pause waiting on Keep going or Stop puts the amber ring on Lines", async () => {
		const paused: TranscriptEvent = { kind: "exchange_paused", id: "x1", ts: 3, withPersonaId: lou.id, withName: lou.name, exchanges: 12, status: "pending" };
		await act(async () => root.render(<PairPane threadKey={key} self={poe} other={lou} row={row("parked")} events={[...events, paused]} people={new Map()} onBack={() => {}} onClose={() => {}} />));
		expect(container.querySelector(".spine-rule[data-status]")?.getAttribute("data-status")).toBe("blocked");
	});

	test("is read only: kept from before the inbox, with nothing that writes to it, stops it or starts it again", async () => {
		// Even where an older desk's record still says live, it is never live here.
		for (const state of ["live", "parked", "closed"] as const) {
			await render(pane(state));
			expect(container.querySelector(".band h2")?.getAttribute("aria-label")).toBe("Pair, earlier");
			expect(container.querySelector(".beat")).toBeNull();
			expect(container.querySelector(".spine-rule[data-status]")?.getAttribute("data-status")).toBe("stopped");
			expect(container.querySelector("textarea, input, [contenteditable]")).toBeNull();
			const keys = [...container.querySelectorAll<HTMLButtonElement>(".band button")].map((one) => one.getAttribute("title"));
			expect(keys.map((key) => key?.split(" ")[0])).toEqual(["Back", "Close"]);
		}
		expect(sent).toEqual([]);
	});

	test("in the conversation that holds it, its link line is a pair's, and shows no kind word", async () => {
		const link: Extract<TranscriptEvent, { kind: "link" }> = { kind: "link", id: "link:pair", ts: 1, thread: key, threadKind: "pair", title: "Lou on grizzly", state: "live" };
		await render(<LinkRow link={link} owner="poe" people={undefined} onOpen={() => {}} />);
		expect(container.querySelector(".link-row")?.getAttribute("data-kind")).toBe("pair");
		expect(text(".link-row")).toBe("Talking with Lou on grizzlyOpen ›");
	});
});

/* ------------------------------------------------------------- command */

describe("task-396 a command left running", () => {
	const runId = "run-1";
	const run: RunningSubagent = { runId, title: "make precheck", startedAt: Date.now() - 63_000, kind: "shell", command: "make precheck" };
	const link = (state: "live" | "closed"): TranscriptEvent => ({
		kind: "link", id: `link:run:${runId}`, ts: run.startedAt, thread: runId, threadKind: "run", personaId: "poe",
		title: "make precheck", state, runKind: "shell", ...(state === "closed" ? { end: "done" as const, elapsedMs: 95_000 } : {}),
	});
	const kept: TranscriptEvent = {
		kind: "tool", id: "tool:c1", ts: 9, toolCallId: "c1", title: "make precheck", toolKind: "execute", status: "completed",
		output: [{ type: "text", text: "precheck: all 8 legs pass in 95s\n" }],
	};
	const stops: number[] = [];
	const pane = (running: RunningSubagent | undefined, events: TranscriptEvent[]) => (
		<CommandPane
			personaId="poe"
			runId={runId}
			owner="Poe"
			cwd="/Users/george/Projects/personal/hotline-wt-049"
			run={running}
			events={events}
			onStop={async () => void stops.push(1)}
			onBack={() => {}}
			onClose={() => {}}
		/>
	);

	test("reads as activity, not a chat: Output over what it wrote, no field, Stop at the foot only where the desk can stop it, and the foot saying whose it is", async () => {
		// desk-threads, decision 2 (George, 2026-11-12).
		output = { command: "make precheck", tail: "restore/full ........ ok\n" };
		stops.length = 0;
		await render(pane(run, [link("live")]));
		expect(text(".spine-rule .spine-rule-title")).toBe("Output");
		expect(container.querySelector(".spine-rule")?.getAttribute("data-status")).toBe("live");
		expect(container.querySelector("textarea, input, .speech")).toBeNull();
		expect(container.querySelector('button[title="Stop this command"]')).toBeNull();
		expect(text(".spine-foot")).toBe("Left running by Poe.");
		await render(pane({ ...run, stoppable: true }, [link("live")]));
		await act(async () => container.querySelector<HTMLButtonElement>('.spine-foot button[title="Stop this command"]')!.click());
		expect(stops).toEqual([1]);
		// Ended, there is nothing to stop.
		await render(pane(undefined, [link("closed")]));
		expect(container.querySelector('button[title="Stop this command"]')).toBeNull();
	});

	test("opens as Command alone in its band, titled by the command, with three facts at most: In, Started", async () => {
		output = { command: "make precheck", tail: "" };
		await render(pane(run, [link("live")]));
		expect(text(".band-kind")).toBe("Command");
		expect(text(".spine-title")).toBe("make precheck");
		const facts = all(".spine-facts dt");
		expect(facts).toEqual(["In", "Started"]);
		expect(text(".spine-facts dd")).toBe("hotline-wt-049");
	});

	test("shows the command line, then its output tail live in a terminal block", async () => {
		output = { command: "make precheck", tail: "==> secrets\nsecrets     pass  0s\n" };
		await render(pane(run, [link("live")]));
		expect(text(".pane-terminal-command")).toBe("$make precheck");
		expect(text(".pane-terminal")).toBe("$make precheck\n==> secrets\nsecrets     pass  0s\n");
		expect(sent).toContainEqual({ cmd: "process.output", params: { personaId: "poe", id: runId } });
		expect(container.querySelector(".band h2")?.getAttribute("aria-label")).toBe("Command, running");
		expect(text(".spine-state")).toMatch(/^Running now1:0\d$/);
		expect(text(".spine-rule-count")).toBe("2 lines");
	});

	test("ended, keeps the last output it read, else the output its run kept", async () => {
		output = { command: "make precheck", tail: "ui          pass  27s\n" };
		await render(pane(run, [link("live")]));
		await render(pane(undefined, [link("closed")]));
		expect(container.querySelector(".band h2")?.getAttribute("aria-label")).toBe("Command, done");
		expect(text(".spine-state")).toBe("Done1:35");
		expect(container.querySelector(".spine-rule")?.getAttribute("data-status")).toBe("stopped");
		expect(text(".pane-terminal")).toBe("$make precheck\nui          pass  27s\n");
		expect(text(".spine-rule-count")).toBe("1 line");
		await act(async () => root.unmount());
		const { createRoot } = await import("react-dom/client");
		root = createRoot(container);
		await render(pane(undefined, [link("closed"), kept]));
		expect(text(".pane-terminal")).toBe("$make precheck\nprecheck: all 8 legs pass in 95s\n");
	});

	test("it has no line in the conversation, and its row at the composer says Command, not Helper", async () => {
		await render(<Transcript personaId="poe" name="Poe" events={[link("live"), link("closed")]} streaming={[]} live={false} focus={null} />);
		expect(container.querySelector(".link-row")).toBeNull();
		expect(container.textContent).not.toContain("make precheck");
		// Its row in the composer's threads panel (task-437), where the chip was.
		const poe = { persona: { id: "poe" }, subagents: [run] } as unknown as Parameters<typeof liveThreads>[0]["entry"];
		const listedRun: ThreadSummary = { thread: { kind: "run", key: runId }, personaId: "poe", title: "make precheck", state: "live", startedAt: run.startedAt, updatedAt: run.startedAt, working: false, waiting: false, runKind: "shell" };
		const [row] = liveThreads({ listed: [listedRun], roster: [], entry: poe, jobs: [], nameOf: () => undefined, now: Date.now() });
		expect(row?.kind).toBe("command");
		expect(row?.line).toBe("Command · make precheck");
	});
});
