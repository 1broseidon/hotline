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
const { SubagentChips } = await import("../src/components/Conversation");
const { linkLine } = await import("../src/links");
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
		<HandoffPane thread={thread} events={events} streaming={streaming} live={live} receiver={{ id: "poe", name: "Poe" }} sender={{ personaId: "elvis", name: "Elvis" }} people={new Map()} />
	);

	test("shows the request as the clamped task card", async () => {
		await render(pane([request, read, reply], false));
		expect(container.querySelector(".pane-task")?.getAttribute("aria-label")).toBe("Request");
		expect(text(".pane-task-body")).toBe(request.kind === "delivery" ? request.text : "");
		expect(text(".pane-task-from")).toEndWith("from Elvis");
		// The request is the card, not a line of the stream as well.
		expect(all(".speech").some((one) => one?.includes("old sign-in"))).toBe(false);
	});

	test("shows the receiver's steps and words live, as in the subagent pane", async () => {
		// Words land a whole bubble at a time, as they do in the conversation.
		await render(pane([request, read], true, [{ messageId: "m2", kind: "agent", text: "Sending Elvis the lines.\n\nThe first", ts: 5 }]));
		expect(container.querySelector(".pane-stream .trail")).not.toBeNull();
		expect(text(".pane-stream .steps-heading")).toBe("Steps · 1");
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

	test('says "◎ Pair" and "Read only", titled "<A> ↔ <B>" with a linked-desk teammate by its desk name', async () => {
		await render(pane("live"));
		expect(text(".pane-kind")).toBe("◎Pair");
		expect(text(".pane-family")).toBe("Read only");
		expect(text(".pane-title")).toBe("Poe ↔ Lou on grizzly");
	});

	test("draws lines on two sides, each with its teammate's face and name", async () => {
		await render(pane("live"));
		const lines = [...container.querySelectorAll(".pair-line")];
		expect(lines.map((line) => line.querySelector(".said-name")?.textContent)).toEqual(["Poe", "Lou on grizzly"]);
		expect(lines.map((line) => line.hasAttribute("data-right"))).toEqual([false, true]);
		expect(lines.map((line) => line.querySelector(".avatar")?.textContent)).toEqual(["P", "L"]);
		expect(lines.map((line) => line.querySelector(".pair-line-said .markdown, .pair-line-said p:last-child")?.textContent)).toEqual([
			"What does the phone say on a sign-in card?",
			"Open Browser and Visit anyway.",
		]);
		// The other teammate's first in the key writes as `user`: the sides follow the key, not the event's kind.
		expect(pairLines(`${lou.id}~${poe.id}`, events, poe, lou).map((line) => (line.kind === "said" ? line.who.name : ""))).toEqual(["Lou on grizzly", "Poe"]);
	});

	test("is live while talking, with Stop exchange in the band while the handoff stands, and none after", async () => {
		await render(pane("live"));
		expect(text(".band h2")).toBe("Talking");
		expect(container.querySelector(".band h2 .beat")).not.toBeNull();
		const stop = container.querySelector<HTMLButtonElement>('button[title="Stop the exchange"]');
		expect(stop?.textContent).toBe("Stop exchange");
		await act(async () => stop!.click());
		expect(sent).toContainEqual({ cmd: "thread.cancel", params: { thread: { kind: "pair", key } } });
		await render(pane("closed"));
		expect(text(".band h2")).toBe("Ended");
		expect(container.querySelector('button[title="Stop the exchange"]')).toBeNull();
	});

	test("in the conversation that holds it, its link line says Pair, not Thread", async () => {
		const link: Extract<TranscriptEvent, { kind: "link" }> = { kind: "link", id: "link:pair", ts: 1, thread: key, threadKind: "pair", title: "Lou on grizzly", state: "live" };
		await render(<LinkRow link={link} owner="poe" people={undefined} onOpen={() => {}} />);
		expect(container.querySelector(".link-row")?.getAttribute("data-kind")).toBe("pair");
		expect(text(".link-row-kind")).toBe("Pair");
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
	const pane = (running: RunningSubagent | undefined, events: TranscriptEvent[]) => (
		<CommandPane personaId="poe" runId={runId} owner="Poe" cwd="/Users/george/Projects/personal/hotline-wt-049" run={running} events={events} onBack={() => {}} onClose={() => {}} />
	);

	test('opens as "$ Command" started by the teammate, titled by the command, "in <dir> · started <time>"', async () => {
		output = { command: "make precheck", tail: "" };
		await render(pane(run, [link("live")]));
		expect(text(".pane-kind")).toBe("$Command");
		expect(text(".pane-family")).toBe("Started by Poe");
		expect(text(".pane-title")).toBe("make precheck");
		expect(text(".pane-meta")).toMatch(/^in hotline-wt-049 · started \d/);
	});

	test("shows the command line, then its output tail live in a terminal block", async () => {
		output = { command: "make precheck", tail: "==> secrets\nsecrets     pass  0s\n" };
		await render(pane(run, [link("live")]));
		expect(text(".pane-command")).toBe("$make precheck");
		expect(text(".pane-terminal")).toBe("==> secrets\nsecrets     pass  0s\n");
		expect(sent).toContainEqual({ cmd: "process.output", params: { personaId: "poe", id: runId } });
		expect(text(".band h2")).toBe("Running");
		expect(text(".band .instrument")).toMatch(/^1:0\d$/);
	});

	test("ended, keeps the last output it read, else the output its run kept", async () => {
		output = { command: "make precheck", tail: "ui          pass  27s\n" };
		await render(pane(run, [link("live")]));
		await render(pane(undefined, [link("closed")]));
		expect(text(".band h2")).toBe("Done");
		expect(text(".band .instrument")).toBe("1:35");
		expect(text(".pane-terminal")).toBe("ui          pass  27s\n");
		await act(async () => root.unmount());
		const { createRoot } = await import("react-dom/client");
		root = createRoot(container);
		await render(pane(undefined, [link("closed"), kept]));
		expect(text(".pane-terminal")).toBe("precheck: all 8 legs pass in 95s\n");
	});

	test("its link line and composer chip say Command, not Subagent", async () => {
		const line = link("closed") as Extract<TranscriptEvent, { kind: "link" }>;
		await render(<LinkRow link={line} owner="poe" people={undefined} onOpen={() => {}} />);
		expect(container.querySelector(".link-row")?.getAttribute("data-kind")).toBe("command");
		expect(text(".link-row-kind")).toBe("Command");
		expect(linkLine(line)).toStartWith("Command · make precheck");
		await render(<SubagentChips subagents={[run]} opened={() => false} onOpenThread={() => {}} />);
		expect(container.querySelector('[aria-label="Command running: make precheck"]')?.textContent).toBe("Command · make precheck");
	});
});
