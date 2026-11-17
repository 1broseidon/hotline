import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { InboxItem, InboxSortedEntry, InboxView, TranscriptEvent } from "../src/generated/contract";
import type { Block } from "../src/scheduledRuns";
import { installDom } from "./dom";

// task-505, one row per ask: "ask Timo what the score of the last game was"
// drew eight things between the person's line and Timo's answer. The desk now
// draws the sent row, the answer and its time: a turn whose only work was
// the inbox lets its row say what came of it, a quiet turn that posts folds
// its inbox line into the path behind its answer, and an answer straight
// under the row it answers does not say whose message it came from.

const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });
const real = await import("../src/voice/transcription");
mock.module("../src/voice/transcription", () => ({ ...real, hearsOnThisMac: async () => true }));
const { wire } = await import("../src/wire");
const { toBlocks } = await import("../src/transcriptFolds");
const { answersUnderTheirRow, foldInboxRuns, groupInbox, sentRowsSay } = await import("../src/scheduledRuns");

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
const outcome = (id: string, message: string, state?: string): TranscriptEvent => ({ kind: "outcome", id, ts: THEN, type: "sent", key: id, message, ...(state !== undefined ? { state } : {}) });

/** The blocks the main conversation draws, by kind and id; a turn's end draws nothing and is left out. */
function shape(blocks: Block[]): string[] {
	return blocks.flatMap((block) => {
		if (block.kind === "event") return block.event.kind === "turn" ? [] : [block.event.id];
		if (block.kind === "steps") return [`steps:${block.id}${block.rowSays === true ? " (row says)" : ""}`];
		if (block.kind === "inbox") return [`row:${block.title}`];
		if (block.kind === "inboxRun") return [`run(${block.read},${block.sent})`];
		return [`${block.kind}:${block.id}`];
	});
}

const drawn = (events: TranscriptEvent[]) => foldInboxRuns(sentRowsSay(groupInbox(toBlocks(events))), new Set());

describe("task-505 rule 2: a turn whose only work was the inbox has no outcome line", () => {
	test("asked, sent and said nothing: no outcome line and no time, the row stands for the turn", () => {
		expect(
			shape(
				drawn([
					ask("u1", "ask timo what the score of the last game was"),
					send("t1"),
					sent("score", "timo", "Score of the last game?"),
					outcome("o1", "Asked Timo for the last game's score"),
					ended("e1"),
				]),
			),
		).toEqual(["u1", "row:Score of the last game?"]);
	});

	test("with a closing bubble, the time stays under the bubble with no outcome on it", () => {
		expect(
			shape(
				drawn([
					ask("u1", "ask timo what the score of the last game was"),
					send("t1"),
					sent("score", "timo", "Score of the last game?"),
					outcome("o1", "Asked Timo for the last game's score"),
					said("a1", "Also, the Devils play again on Thursday."),
					ended("e1"),
				]),
			),
		).toEqual(["u1", "row:Score of the last game?", "a1", "steps:t1 (row says)"]);
	});

	test("a reaction and a post are no work: the row still says it", () => {
		expect(
			shape(drawn([ask("u1", "ask timo"), tool("t0", "react", "react"), send("t1"), sent("score", "timo", "Score?"), outcome("o1", "Asked Timo"), ended("e1")])),
		).toEqual(["u1", "row:Score?"]);
	});

	test("what reaching a colleague takes on the way is no work: a look at the team, the room's skill, Claude Code loading its tools", () => {
		const looked = (id: string, title: string, key: string, one: string): TranscriptEvent => ({ ...tool(id, title, key), said: { key, one, many: one, doing: one } } as TranscriptEvent);
		// Hotline Agent, as George's own tape has it.
		const hotlineAgent = [ask("u1", "ask timo"), tool("r1", "read .agents/skills/hotline-room/SKILL.md", "read"), tool("r2", "list_teammates", "team"), send("t1"), sent("score", "timo", "Score?"), outcome("o1", "Asked Timo"), ended("e1")];
		expect(shape(drawn(hotlineAgent))).toEqual(["u1", "row:Score?"]);
		const claudeCode = [ask("u1", "ask timo"), looked("r1", "ToolSearch", "tools", "Looked up its tools"), send("t1"), sent("score", "timo", "Score?"), outcome("o1", "Asked Timo"), ended("e1")];
		expect(shape(drawn(claudeCode))).toEqual(["u1", "row:Score?"]);
		// Another file read, or a tool used through call_tool, is work.
		const read = [ask("u1", "ask timo"), tool("r1", "read notes/games.md", "read"), send("t1"), sent("score", "timo", "Score?"), outcome("o1", "Asked Timo"), ended("e1")];
		expect(shape(drawn(read))).toEqual(["u1", "row:Score?", "steps:r1"]);
		const used = [ask("u1", "ask timo"), looked("r1", "call_tool", "tools", "Used a tool"), send("t1"), sent("score", "timo", "Score?"), outcome("o1", "Asked Timo"), ended("e1")];
		expect(shape(drawn(used))).toEqual(["u1", "row:Score?", "steps:r1"]);
	});

	test("other work in the turn keeps its outcome line, and so does a failure", () => {
		const searched = drawn([
			ask("u1", "find the score and send it to timo"),
			tool("t0", "web_search: devils score", "search"),
			send("t1"),
			sent("score", "timo", "Devils won 7-4"),
			outcome("o1", "Sent Timo the score"),
			ended("e1"),
		]);
		expect(shape(searched)).toEqual(["u1", "row:Devils won 7-4", "steps:t0"]);
		const failed = drawn([ask("u1", "ask timo and huck"), send("t1"), sent("score", "timo", "Score?"), outcome("o1", "Could not reach Huck", "failed"), ended("e1")]);
		expect(shape(failed)).toEqual(["u1", "row:Score?", "steps:t1"]);
	});

	test("a turn with no sent row keeps its line: reading the inbox alone is not the row's to say", () => {
		expect(shape(drawn([ask("u1", "anything new?"), tool("t1", "read_inbox", "colleague"), outcome("o1", "Nothing new"), said("a1", "Nothing new."), ended("e1")]))).toEqual([
			"u1",
			"a1",
			"steps:t1",
		]);
	});

	test("what the desk keeps is untouched: the steps still hold the outcome, for the pane and the phone", () => {
		const blocks = groupInbox(toBlocks([ask("u1", "ask timo"), send("t1"), sent("score", "timo", "Score?"), outcome("o1", "Asked Timo"), said("a1", "Thursday too."), ended("e1")]));
		const steps = sentRowsSay(blocks).find((block) => block.kind === "steps");
		expect(steps?.kind === "steps" && steps.items.map((item) => item.id)).toEqual(["t1", "o1"]);
	});
});

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

describe("task-505 drawn", () => {
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

	const draw = async (events: TranscriptEvent[]) => {
		const { Transcript } = await import("../src/components/Transcript");
		await act(async () => root!.render(<Transcript personaId="goldie" name="Goldie" people={people} events={events} streaming={[]} live={false} focus={null} onOpenInboxItem={() => {}} onOpenWork={() => {}} />));
	};

	test("rule 2: the asked turn is its row alone, Answered, with no outcome line or time under it", async () => {
		await draw([ask("u1", "ask timo what the score of the last game was"), send("t1"), sent("score", "timo", "Score of the last game?"), outcome("o1", "Asked Timo for the last game's score"), ended("e1")]);
		expect([...container.querySelectorAll(".link-row[data-kind=inbox]")].map((row) => row.querySelector(".link-row-words")?.textContent)).toEqual(["Score of the last game?· To Timo · Answered"]);
		expect(container.querySelector(".steps-caption")).toBeNull();
		expect(container.textContent).not.toContain("Asked Timo for the last game's score");
	});

	test("rule 4 with 2 and 3: the whole ask is three things, the row, the answer and its time", async () => {
		await draw([ask("u1", "ask timo what the score of the last game was"), send("t1"), sent("score", "timo", "Score of the last game?"), outcome("o1", "Asked Timo for the last game's score"), ended("e1"), ...answerTurn]);
		expect(container.querySelectorAll(".link-row[data-kind=inbox]")).toHaveLength(1);
		expect(container.querySelectorAll(".speech.said-them")).toHaveLength(1);
		expect(container.querySelectorAll(".steps-caption")).toHaveLength(1);
		expect(container.querySelector("[data-kind=inbox-from]")).toBeNull();
		expect(container.querySelector("[data-kind=inbox-run]")).toBeNull();
		// In that order, under the person's line.
		const order = [...container.querySelectorAll(".speech.said-me, .link-row[data-kind=inbox], .speech.said-them, .steps-caption")].map((one) => one.className.split(" ")[0]);
		expect(order).toEqual(["speech", "link-row", "speech", "steps-caption"]);
	});

	test("rule 4: the answer after other talk keeps From Timo's message", async () => {
		await draw([
			ask("u1", "ask timo what the score of the last game was"),
			send("t1"),
			sent("score", "timo", "Score of the last game?"),
			ended("e1"),
			ask("u2", "while we wait, is the release branch green?"),
			said("a2", "Yes. All three builds passed."),
			ended("e3"),
			...answerTurn,
		]);
		const from = [...container.querySelectorAll(".link-row[data-kind=inbox-from]")];
		expect(from.map((row) => row.querySelector(".link-row-words")?.textContent)).toEqual(["From Timo's message ·Re: Score of the last game?"]);
		expect(from[0]?.closest(".said-group")?.textContent).toContain("Timo says the Devils beat the Capitals");
	});

	test("rule 2: a closing bubble keeps its time line, with no outcome on it", async () => {
		await draw([ask("u1", "ask timo"), send("t1"), sent("score", "timo", "Score of the last game?"), outcome("o1", "Asked Timo for the last game's score"), said("a1", "Also, they play again on Thursday."), ended("e1")]);
		const caption = container.querySelector(".steps-caption");
		expect(caption).not.toBeNull();
		expect(caption?.querySelector(".outcome")).toBeNull();
		expect(container.textContent).not.toContain("Asked Timo for the last game's score");
	});
});

/** A sort of one item. */
const sorting = (id: string, entry: InboxSortedEntry): TranscriptEvent => ({ kind: "inbox_sorted", id, ts: THEN, open: 0, unread: 0, entries: [entry] });
/** Timo's answer, as Goldie's inbox sort leaves it. */
const answered = (id: string) => sorting(id, { item: "reply", title: "Re: Score of the last game?", from: "timo", state: "closed", outcome: "answered" });
const post = (id: string) => tool(id, "post_to_chat", "post");
const readInbox = (id: string) => tool(id, "read_inbox", "colleague");
const sortInbox = (id: string) => tool(id, "update_inbox", "colleague");
/** The quiet turn Timo's answer woke, which posts it. */
const answerTurn: TranscriptEvent[] = [readInbox("q1"), sortInbox("q2"), answered("s1"), post("q3"), said("p1", "Timo says the Devils beat the Capitals 7–4.", "reply"), ended("e2", true)];

describe("task-505 rule 3: the quiet inbox line folds into the answer", () => {
	test("a quiet turn that posts draws no inbox line: its bubble and its time", () => {
		expect(shape(drawn([ended("e0"), ...answerTurn]))).toEqual(["p1", "steps:q1"]);
	});

	test("a quiet turn that posts nothing keeps its line exactly as today", () => {
		expect(shape(drawn([ended("e0"), readInbox("q1"), sortInbox("q2"), answered("s1"), ended("e2", true)]))).toEqual(["run(1,0)"]);
	});

	test("an earlier quiet turn's line stays, and only the posting turn's rows leave it", () => {
		const earlier = [readInbox("r1"), sorting("s0", { item: "notes", title: "Storage numbers", from: "timo", state: "later", reason: "after the release" }), ended("e1", true)];
		expect(shape(drawn([ended("e0"), ...earlier, ...answerTurn]))).toEqual(["run(1,0)", "p1", "steps:q1"]);
	});

	test("while the quiet turn still runs, its posted answer already stands alone", () => {
		expect(shape(drawn([ended("e0"), readInbox("q1"), sortInbox("q2"), answered("s1"), post("q3"), said("p1", "Timo says 7–4.", "reply")]))).toEqual(["p1", "steps:q1"]);
	});

	test("a turn the person spoke into is theirs: the sort before their line keeps its line", () => {
		expect(shape(drawn([ended("e0"), readInbox("q1"), answered("s1"), ask("u1", "what did timo say?"), said("a1", "7–4 to the Devils."), ended("e2")]))).toEqual(["run(1,0)", "steps:q1", "u1", "a1"]);
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

	test("Inbox, a section of the path, has the row the chat's line would have opened to, and a press opens the item", async () => {
		const { WorkPage } = await import("../src/components/Work");
		tape = [ended("e0"), ...answerTurn];
		const opened: string[] = [];
		await act(async () =>
			root!.render(<WorkPage personaId="goldie-pane" blockId={null} name="Goldie" live={false} resting={false} people={people} onOpenThread={() => {}} onOpenItem={(id) => opened.push(id)} onClose={() => {}} />),
		);
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
		const section = container.querySelector("section[aria-label='Inbox']");
		expect(section?.querySelector(".spine-rule-title")?.textContent).toBe("Inbox");
		const rows = [...(section?.querySelectorAll<HTMLButtonElement>(".list-row") ?? [])];
		expect(rows.map((row) => [row.querySelector(".list-title-text")?.textContent, row.querySelector(".list-line")?.textContent])).toEqual([["Re: Score of the last game?", "From Timo · Answered"]]);
		// Before the steps, as Files changed is.
		expect(section?.nextElementSibling?.getAttribute("aria-label")).toBe("Steps");
		await act(async () => void rows[0]!.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
		expect(opened).toEqual(["reply"]);
	});
});

describe("task-505 rule 4: From Timo's message only where it is needed", () => {
	const reply = item("reply", { from: "timo", to: "goldie", title: "Re: Score of the last game?", replyTo: "score" });
	const known = new Map([[reply.id, reply]]);
	const asked = [ask("u1", "ask timo what the score of the last game was"), send("t1"), sent("score", "timo", "Score of the last game?"), outcome("o1", "Asked Timo"), ended("e1")];
	const under = (events: TranscriptEvent[], items: ReadonlyMap<string, InboxItem> = known) => [...answersUnderTheirRow(drawn(events), items)];

	test("an answer straight under the row it answers draws no line, the turn's end and the folded inbox line between them", () => {
		expect(under([...asked, ...answerTurn])).toEqual(["p1"]);
	});

	test("an answer that comes after other talk keeps its line", () => {
		expect(under([...asked, ask("u2", "while we wait, is the release branch green?"), said("a2", "Yes. All three builds passed."), ended("e3"), ...answerTurn])).toEqual([]);
	});

	test("an answer under a closing bubble and its time keeps its line: those are drawn between", () => {
		const closing = [ask("u1", "ask timo"), send("t1"), sent("score", "timo", "Score of the last game?"), said("a1", "Also, they play Thursday."), ended("e1")];
		expect(under([...closing, ...answerTurn])).toEqual([]);
	});

	test("an answer that cannot be linked to a row in view keeps its line", () => {
		// No reply-to link on the item.
		const { replyTo: _none, ...unlinked } = reply;
		expect(under([...asked, ...answerTurn], new Map([[reply.id, unlinked]]))).toEqual([]);
		// The teammate's inbox does not know the item yet.
		expect(under([...asked, ...answerTurn], new Map())).toEqual([]);
		// The row is not in the loaded transcript.
		expect(under([ended("e1"), ...answerTurn])).toEqual([]);
		// It answers another item than the row above.
		expect(under([...asked, ...answerTurn], new Map([[reply.id, { ...reply, replyTo: "another" }]]))).toEqual([]);
	});

	test("a file keeps its line always; only words change", () => {
		const file: TranscriptEvent = { kind: "agent", id: "f1", ts: THEN, text: "", fromItem: "reply", attachments: [{ name: "score.png", mime: "image/png", size: 10 } as never] };
		expect(under([...asked, readInbox("q1"), answered("s1"), file, ended("e2", true)])).toEqual([]);
	});

	test("an answer of several bubbles is one answer under its row", () => {
		expect(under([...asked, readInbox("q1"), answered("s1"), said("p1", "Timo says 7–4.", "reply"), said("p1-2", "At Washington.", "reply"), ended("e2", true)])).toEqual(["p1", "p1-2"]);
	});
});
