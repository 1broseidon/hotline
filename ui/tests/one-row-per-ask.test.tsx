import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { InboxItem, InboxSortedEntry, InboxView, TranscriptEvent } from "../src/generated/contract";
import type { Block } from "../src/scheduledRuns";
import { installDom } from "./dom";

// Quiet turns that post an answer fold their inbox rows into the turn's Steps
// and its path's Teammates section, and an answer bubble has no
// "From <teammate>'s message" line over it (George, 2026-11-21: to the
// person, inbox items are not mail).

const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });
const real = await import("../src/voice/transcription");
mock.module("../src/voice/transcription", () => ({ ...real, hearsOnThisMac: async () => true }));
const { wire } = await import("../src/wire");
const { toBlocks } = await import("../src/transcriptFolds");
const { groupInbox, withoutUnsaidTurns } = await import("../src/scheduledRuns");

const NOW = Date.now();
/** When the lines were said: long enough ago that the chat's pacing shows every bubble at once. */
const THEN = NOW - 300_000;
const people = new Map([
	["goldie", { name: "Goldie" }],
	["timo", { name: "Timo" }],
]);

const ask = (id: string, text: string): TranscriptEvent => ({ kind: "user", id, ts: THEN, text });
const said = (id: string, text: string, fromItem?: string): TranscriptEvent => ({ kind: "agent", id, ts: THEN, text, ...(fromItem !== undefined ? { fromItem } : {}) });
const ended = (id: string, quiet = false): TranscriptEvent => ({ kind: "turn", id, ts: THEN, stopReason: "end_turn", ...(quiet ? { quiet: true } : {}) });
const sent = (item: string, to: string, title: string): TranscriptEvent => ({ kind: "inbox_sent", id: `inbox-sent:${item}`, ts: THEN, item, to, title });
/** A tool call as the desk records it, said in the group the window knows it by. */
const tool = (id: string, title: string, key: string): TranscriptEvent => ({
	kind: "tool",
	id,
	ts: THEN,
	toolCallId: id,
	title,
	status: "completed",
	said: { key, one: title, many: title, doing: title },
});
const send = (id: string) => tool(id, "send", "colleague");

/** The blocks the main conversation draws, by kind and id; a turn's end draws nothing and is left out. */
function shape(blocks: Block[]): string[] {
	return blocks.flatMap((block) => {
		if (block.kind === "event") return block.event.kind === "turn" ? [] : [block.event.id];
		if (block.kind === "steps") return [`steps:${block.id}`];
		if (block.kind === "inbox") return [`row:${block.title}`];
		return [`${block.kind}:${block.id}`];
	});
}

const drawn = (events: TranscriptEvent[]) => withoutUnsaidTurns(groupInbox(toBlocks(events)), new Set());

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

let act: typeof import("react").act;
let root: Root | null;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

function item(id: string, more: Partial<InboxItem>): InboxItem {
	return {
		id,
		ts: NOW - 60_000,
		from: "goldie",
		fromThread: { kind: "dm", key: "goldie" },
		to: "timo",
		title: "Score of the last game?",
		body: "",
		wants: "answer",
		state: "closed",
		outcome: "answered",
		receipts: [{ state: "new", at: NOW - 60_000 }],
		senderTold: 1,
		...more,
	};
}

/** A sort of one item. */
const sorting = (id: string, entry: InboxSortedEntry): TranscriptEvent => ({ kind: "inbox_sorted", id, ts: THEN, open: 0, unread: 0, entries: [entry] });
/** Timo's answer, as Goldie's inbox sort leaves it. */
const answered = (id: string) => sorting(id, { item: "reply", title: "Re: Score of the last game?", from: "timo", state: "closed", outcome: "answered" });
const post = (id: string) => tool(id, "post_to_chat", "post");
const readInbox = (id: string) => tool(id, "read_inbox", "colleague");
const sortInbox = (id: string) => tool(id, "update_inbox", "colleague");
/** The quiet turn Timo's answer woke, which posts it. */
const answerTurn: TranscriptEvent[] = [readInbox("q1"), sortInbox("q2"), answered("s1"), post("q3"), said("p1", "Timo says the Devils beat the Capitals 7–4.", "reply"), ended("e2", true)];

describe("an answer bubble has no From Timo's message line and no chat-level sent row", () => {
	beforeEach(async () => {
		({ teardown } = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} }));
		act = (await import("react")).act;
		const { createRoot } = await import("react-dom/client");
		container = document.createElement("div");
		document.body.append(container);
		root = createRoot(container);
		const reply = item("reply", { from: "timo", to: "goldie", title: "Re: Score of the last game?", replyTo: "score" });
		const view: InboxView = { personaId: "goldie", open: [], closed: [reply], sent: [item("score", {})], unread: 0 };
		spyOn(wire, "subscribe").mockImplementation(((target: { inbox?: string }, handlers: { snapshot(items: unknown): void }) => {
			if (target.inbox === "goldie") handlers.snapshot(view);
			return () => {};
		}) as typeof wire.subscribe);
	});
	afterEach(async () => {
		await act(async () => root?.unmount());
		root = null;
		await teardown();
	});

	test("an answer bubble straight after an ask turn stands alone, with no line naming whose message it was", async () => {
		const { Transcript } = await import("../src/components/Transcript");
		const events = [ask("u1", "ask timo what the score of the last game was"), send("t1"), sent("score", "timo", "Score of the last game?"), ended("e1"), ...answerTurn];
		await act(async () => root!.render(<Transcript personaId="goldie" name="Goldie" people={people} events={events} streaming={[]} live={false} focus={null} onOpenInboxItem={() => {}} onOpenWork={() => {}} />));
		expect(container.querySelectorAll(".link-row[data-kind=inbox]")).toHaveLength(0);
		expect(container.querySelectorAll(".link-row[data-kind=inbox-from]")).toHaveLength(0);
		expect(container.textContent).not.toContain("From Timo's message");
		expect(container.textContent).toContain("Timo says the Devils beat the Capitals");
	});
});

describe("task-505 rule 3: a quiet turn's inbox is the answer's, and a turn that said nothing draws nothing", () => {
	test("a quiet turn that posts draws no inbox line: its bubble and its time", () => {
		expect(shape(drawn([ended("e0"), ...answerTurn]))).toEqual(["p1", "steps:q1"]);
	});

	// George, 2026-11-21: the chat never shows a standalone Inbox line, not even for a turn with no bubble.
	test("a quiet turn that posts nothing draws nothing", () => {
		expect(shape(drawn([ended("e0"), readInbox("q1"), sortInbox("q2"), answered("s1"), ended("e2", true)]))).toEqual([]);
	});

	test("an earlier quiet turn draws nothing, and the posting turn keeps its bubble and its time", () => {
		const earlier = [readInbox("r1"), sorting("s0", { item: "notes", title: "Storage numbers", from: "timo", state: "later", reason: "after the release" }), ended("e1", true)];
		expect(shape(drawn([ended("e0"), ...earlier, ...answerTurn]))).toEqual(["p1", "steps:q1"]);
	});

	test("while the quiet turn still runs, its posted answer already stands alone", () => {
		expect(shape(drawn([ended("e0"), readInbox("q1"), sortInbox("q2"), answered("s1"), post("q3"), said("p1", "Timo says 7–4.", "reply")]))).toEqual(["p1", "steps:q1"]);
	});

	test("a turn the person spoke into is theirs: what it sorted before their line is on their answer's line", () => {
		expect(shape(drawn([ended("e0"), readInbox("q1"), answered("s1"), ask("u1", "what did timo say?"), said("a1", "7–4 to the Devils."), ended("e2")]))).toEqual(["steps:q1", "u1", "a1"]);
	});

	test("the rows it would have opened to are the path's, behind the answer's time", async () => {
		const { turnInbox } = await import("../src/inbox");
		const events = [ask("u1", "ask timo what the score was"), send("t1"), sent("score", "timo", "Score of the last game?"), ended("e1"), ...answerTurn];
		const steps = toBlocks(events).filter((block) => block.kind === "steps");
		const answer = steps[steps.length - 1]!;
		expect(answer.kind === "steps" && turnInbox(events, answer.items.map((item) => item.id)).map((row) => (row.kind === "sorted" ? row.entry.title : row.sent.title))).toEqual([
			"Re: Score of the last game?",
		]);
		// The asked turn's path lists what it sent.
		const asked = steps[0]!;
		expect(asked.kind === "steps" && turnInbox(events, asked.items.map((item) => item.id)).map((row) => row.kind)).toEqual(["sent"]);
	});
});

describe("task-505 rule 3 in the pane: the answer's time opens a path that lists its inbox", () => {
	let restores: (() => void)[] = [];
	let tape: TranscriptEvent[] = [];
	beforeEach(async () => {
		restores = [];
		const installed = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
		teardown = installed.teardown;
		Object.assign(globalThis, { HTMLElement: installed.dom.HTMLElement });
		restores.push(() => Reflect.deleteProperty(globalThis, "HTMLElement"));
		const { replaceDesks, wireFor } = await import("../src/desks");
		replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
		restores.push(() => replaceDesks([]));
		const connection = wireFor("local");
		const open = spyOn(connection, "onConnection").mockImplementation((watcher) => {
			watcher("open");
			return () => {};
		});
		restores.push(() => open.mockRestore());
		const view: InboxView = { personaId: "goldie-pane", open: [], closed: [item("reply", { from: "timo", to: "goldie-pane", title: "Re: Score of the last game?" })], sent: [], unread: 0 };
		const subscription = spyOn(connection, "subscribe").mockImplementation(((target: { inbox?: string }, handlers: { snapshot(items: unknown): void }) => {
			queueMicrotask(() => handlers.snapshot(target.inbox !== undefined ? view : tape));
			return () => {};
		}) as typeof connection.subscribe);
		restores.push(() => subscription.mockRestore());
		act = (await import("react")).act;
		const { createRoot } = await import("react-dom/client");
		container = document.createElement("div");
		document.body.append(container);
		root = createRoot(container);
	});
	afterEach(async () => {
		await act(async () => root?.unmount());
		root = null;
		for (const restore of restores.reverse()) restore();
		await teardown();
	});

	test("Teammates, a section of the path, has the row the chat's line would have opened to, and a press opens the item", async () => {
		const { WorkPage } = await import("../src/components/Work");
		tape = [ended("e0"), ...answerTurn];
		const opened: string[] = [];
		await act(async () =>
			root!.render(<WorkPage personaId="goldie-pane" blockId={null} name="Goldie" live={false} resting={false} people={people} onOpenThread={() => {}} onOpenItem={(id) => opened.push(id)} onClose={() => {}} />),
		);
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
		const section = container.querySelector("section[aria-label='Teammates']");
		expect(section?.querySelector(".spine-rule-title")?.textContent).toBe("Teammates");
		expect([...container.querySelectorAll(".spine-rule-title")].some((title) => title.textContent === "Inbox")).toBe(false);
		const rows = [...(section?.querySelectorAll<HTMLButtonElement>(".list-row") ?? [])];
		expect(rows.map((row) => [row.querySelector(".list-title-text")?.textContent, row.querySelector(".list-line")?.textContent])).toEqual([["Re: Score of the last game?", "From Timo · Answered"]]);
		// Before the steps, as Files changed is.
		expect(section?.nextElementSibling?.getAttribute("aria-label")).toBe("Steps");
		await act(async () => void rows[0]!.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		expect(opened).toEqual(["reply"]);
	});
});
