import { describe, expect, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";

// The store reaches for the window's desks; folding needs none of them.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) }, localStorage: { getItem: () => null, setItem: () => {} } });
const { append, dmOf, NOTHING_YET, reduceThread, sameThread, settle } = await import("../src/tape");
type Fold = Parameters<typeof reduceThread>[1];

const said = (id: string, text = id, extra: object = {}) => ({ kind: "agent", id, ts: 1, text, ...extra }) as TranscriptEvent;
const tool = (id: string, status: string) => ({ kind: "tool", id, ts: 1, title: "read", status }) as unknown as TranscriptEvent;
const run = (folds: Fold[]) => folds.reduce(reduceThread, NOTHING_YET);
const words = (messageId: string, text: string, kind: "text" | "thought" = "text"): Fold => ({
	type: "words",
	deltas: [{ type: "thread_delta", thread: dmOf("p"), messageId, kind, text }],
});

describe("a thread's lines, whatever its kind", () => {
	test("a snapshot opens it, folded by id", () => {
		const held = run([{ type: "snapshot", items: [said("a"), tool("t", "pending"), tool("t", "completed")] }]);
		expect(held.loaded).toBe(true);
		expect(held.events.map((one) => one.id)).toEqual(["a", "t"]);
		expect(held.events[1]).toMatchObject({ status: "completed" });
		expect(held.more).toBe(false);
	});

	test("an event lands by id: new at the bottom, a rewrite in place", () => {
		const held = run([
			{ type: "snapshot", items: [said("a"), said("b")] },
			{ type: "event", item: said("c") },
			{ type: "event", item: said("a", "rewritten") },
		]);
		expect(held.events.map((one) => one.id)).toEqual(["a", "b", "c"]);
		expect(held.events[0]).toMatchObject({ text: "rewritten" });
	});

	test("a full window may have more above it, a short one is the whole thread", () => {
		const many = Array.from({ length: 200 }, (_, at) => said(`m${at}`));
		expect(run([{ type: "snapshot", items: many }]).more).toBe(true);
		expect(run([{ type: "snapshot", items: many.slice(1) }]).more).toBe(false);
	});

	test("a reconnect's second snapshot keeps the pages loaded above the window", () => {
		const held = run([
			{ type: "snapshot", items: [said("c"), said("d")] },
			{ type: "page", events: [said("a"), said("b")], more: true },
			{ type: "snapshot", items: [said("c"), said("d"), said("e")] },
		]);
		expect(held.events.map((one) => one.id)).toEqual(["a", "b", "c", "d", "e"]);
		expect(held.more).toBe(true);
	});

	test("a page adds only lines not already held, and says whether more remain", () => {
		const held = run([
			{ type: "snapshot", items: [said("c")] },
			{ type: "page", events: [said("b"), said("c")], more: false },
		]);
		expect(held.events.map((one) => one.id)).toEqual(["b", "c"]);
		expect(held.more).toBe(false);
	});

	test("a rewrite of a line above the window is not new, and is not drawn at the bottom", () => {
		const above = { ...said("old"), ts: 0 } as TranscriptEvent;
		const many = Array.from({ length: 200 }, (_, at) => ({ ...said(`m${at}`), ts: 10 + at }) as TranscriptEvent);
		const held = run([{ type: "snapshot", items: many }, { type: "event", item: above }]);
		expect(held.events.some((one) => one.id === "old")).toBe(false);
	});
});

describe("words arriving", () => {
	test("stream as a bubble of their kind, growing, until the line that carries them lands", () => {
		let held = run([{ type: "snapshot", items: [] }, words("m1", "hel"), words("m1", "lo"), words("th", "hmm", "thought")]);
		expect(held.streaming).toEqual([
			{ messageId: "m1", kind: "agent", text: "hello" },
			{ messageId: "th", kind: "thought", text: "hmm" },
		]);
		held = reduceThread(held, { type: "event", item: said("m1", "hello") });
		expect(held.streaming.map((one) => one.messageId)).toEqual(["th"]);
	});

	test("several deltas in one frame fold in order", () => {
		const deltas = ["a", "b", "c"].map((text) => ({ type: "thread_delta" as const, thread: dmOf("p"), messageId: "m", kind: "text" as const, text }));
		expect(reduceThread(NOTHING_YET, { type: "words", deltas }).streaming).toEqual([{ messageId: "m", kind: "agent", text: "abc" }]);
		expect(reduceThread(NOTHING_YET, { type: "words", deltas: [] })).toBe(NOTHING_YET);
	});

	test("a snapshot drops what was mid-stream: it is in the snapshot or will stream again", () => {
		const held = run([words("m", "half"), { type: "snapshot", items: [said("m", "half done")] }]);
		expect(held.streaming).toEqual([]);
	});

	test("the rest of a streamed reply whose first bubble landed stays under the bubbles' ids", () => {
		const one = "The first paragraph says enough to stand as a bubble of its own, with a second sentence.";
		const two = "The second paragraph is as long as the first, so that it is not folded into it.";
		const live = append([], { type: "thread_delta", thread: dmOf("p"), messageId: "m", kind: "text", text: `${one}\n\n${two}` });
		const next = settle(live, said("m", one));
		expect(next).toEqual([{ messageId: "m-2", kind: "agent", text: two, bubbleOf: { base: "m", index: 1 } }]);
	});
});

describe("what happens to a thread nobody is reading", () => {
	test("leaving drops what was mid-stream, and a download, and nothing else", () => {
		const held = run([{ type: "snapshot", items: [said("a")] }, words("m", "x"), { type: "pull", pulling: { done: 1, total: 3 } }]);
		const left = reduceThread(held, { type: "left" });
		expect(left.streaming).toEqual([]);
		expect(left.pulling).toBeNull();
		expect(left.events).toBe(held.events);
		expect(reduceThread(left, { type: "left" })).toBe(left);
	});

	test("a download is a computer's, drawn on its button", () => {
		expect(reduceThread(NOTHING_YET, { type: "pull", pulling: { done: 0, total: 0 } }).pulling).toEqual({ done: 0, total: 0 });
	});
});

test("two names are one thread when kind and key agree", () => {
	expect(sameThread({ kind: "side", key: "s" }, { kind: "side", key: "s" })).toBe(true);
	expect(sameThread({ kind: "side", key: "s" }, { kind: "run", key: "s" })).toBe(false);
	expect(dmOf("ada")).toEqual({ kind: "dm", key: "ada" });
});
