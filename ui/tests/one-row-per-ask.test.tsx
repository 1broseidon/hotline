import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { InboxItem, InboxView, TranscriptEvent } from "../src/generated/contract";
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
const { foldInboxRuns, groupInbox, sentRowsSay } = await import("../src/scheduledRuns");

const NOW = Date.now();
const people = new Map([
	["goldie", { name: "Goldie" }],
	["timo", { name: "Timo" }],
]);

const ask = (id: string, text: string): TranscriptEvent => ({ kind: "user", id, ts: NOW, text });
const said = (id: string, text: string, fromItem?: string): TranscriptEvent => ({ kind: "agent", id, ts: NOW, text, ...(fromItem !== undefined ? { fromItem } : {}) });
const ended = (id: string, quiet = false): TranscriptEvent => ({ kind: "turn", id, ts: NOW, stopReason: "end_turn", ...(quiet ? { quiet: true } : {}) });
const sent = (item: string, to: string, title: string): TranscriptEvent => ({ kind: "inbox_sent", id: `inbox-sent:${item}`, ts: NOW, item, to, title });
/** A tool call as the desk records it, said in the group the window knows it by. */
const tool = (id: string, title: string, key: string): TranscriptEvent => ({
	kind: "tool",
	id,
	ts: NOW,
	toolCallId: id,
	title,
	status: "completed",
	said: { key, one: title, many: title, doing: title },
});
const send = (id: string) => tool(id, "send", "colleague");
const outcome = (id: string, message: string, state?: string): TranscriptEvent => ({ kind: "outcome", id, ts: NOW, type: "sent", key: id, message, ...(state !== undefined ? { state } : {}) });

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
		const view: InboxView = { personaId: "goldie", open: [], closed: [], sent: [item("score", {})], unread: 0 };
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

	test("rule 2: a closing bubble keeps its time line, with no outcome on it", async () => {
		await draw([ask("u1", "ask timo"), send("t1"), sent("score", "timo", "Score of the last game?"), outcome("o1", "Asked Timo for the last game's score"), said("a1", "Also, they play again on Thursday."), ended("e1")]);
		const caption = container.querySelector(".steps-caption");
		expect(caption).not.toBeNull();
		expect(caption?.querySelector(".outcome")).toBeNull();
		expect(container.textContent).not.toContain("Asked Timo for the last game's score");
	});
});
