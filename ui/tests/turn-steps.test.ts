import { describe, expect, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";
import { stepRuns, toBlocks, withStreaming } from "../src/transcriptFolds";

const user = (id: string, ts: number): TranscriptEvent => ({ kind: "user", id, ts, text: "plan the release" });
const agent = (id: string, ts: number): TranscriptEvent => ({ kind: "agent", id, ts, text: `said ${id}` });
const tool = (id: string, ts: number): TranscriptEvent => ({ kind: "tool", id, ts, toolCallId: id, title: `step ${id}`, status: "completed" });
const thought = (id: string, ts: number): TranscriptEvent => ({ kind: "thought", id, ts, text: "hmm" });
const turn = (id: string, ts: number): TranscriptEvent => ({ kind: "turn", id, ts, stopReason: "end_turn" });

const shape = (blocks: ReturnType<typeof toBlocks>) =>
	blocks.map((block) => (block.kind === "steps" ? `steps(${block.items.map((one) => one.id).join(",")})${block.open ? " open" : ""}` : block.kind === "event" ? block.event.id : block.kind));

describe("a turn's steps", () => {
	test("wherever they fell between its bubbles, they are one block under its last bubble, named by the first", () => {
		const blocks = toBlocks([user("u", 1), thought("t1", 2), agent("a1", 3), tool("s1", 4), tool("s2", 5), agent("a2", 6), turn("end", 7)]);
		expect(shape(blocks)).toEqual(["u", "a1", "a2", "steps(t1,s1,s2)", "end"]);
		const steps = blocks[3]!;
		expect(steps.kind === "steps" && steps.id).toBe("t1");
		// Stamped as the bubble above it, so it never draws a time line of its own.
		expect(steps.kind === "steps" && steps.ts).toBe(6);
	});

	test("a turn still running has its steps last and open, and a thought being written joins them", () => {
		const blocks = toBlocks([user("u", 1), tool("s1", 2), agent("a1", 3), tool("s2", 4)]);
		expect(shape(blocks)).toEqual(["u", "a1", "steps(s1,s2) open"]);
		const live = withStreaming(blocks, [{ messageId: "a2", kind: "agent", ts: 5, text: "A whole paragraph that is long enough to stand as a bubble of its own.\n\n" }, { messageId: "t9", kind: "thought", ts: 6, text: "next" }]);
		expect(shape(live)).toEqual(["u", "a1", "steps(s1,s2,t9) open", "a2"]);
	});

	test("each turn has its own, and a message from the person ends one that never wrote its end", () => {
		const blocks = toBlocks([user("u1", 1), tool("s1", 2), agent("a1", 3), user("u2", 4), tool("s2", 5), agent("a2", 6), turn("end", 7)]);
		expect(shape(blocks)).toEqual(["u1", "a1", "steps(s1)", "u2", "a2", "steps(s2)", "end"]);
	});

	test("a call's transcript keeps each run of steps where it fell", () => {
		const blocks = toBlocks([user("u", 1), tool("s1", 2), agent("a1", 3), tool("s2", 4), agent("a2", 5), turn("end", 6)], true);
		expect(shape(blocks)).toEqual(["u", "steps(s1)", "a1", "steps(s2)", "a2", "end"]);
	});

	test("a run says whether its turn is still open, so a turn just begun is never taken for more of the last", () => {
		const ended = stepRuns([user("u1", 1), tool("s1", 2), agent("a1", 3), turn("end", 4), user("u2", 5)], []);
		expect(ended.map((run) => [run.id, run.open === true])).toEqual([["s1", false]]);
		const begun = stepRuns([user("u1", 1), tool("s1", 2), agent("a1", 3), turn("end", 4), user("u2", 5)], [{ messageId: "t2", kind: "thought", ts: 6, text: "thinking" }]);
		expect(begun.map((run) => [run.id, run.open === true])).toEqual([["s1", false], ["t2", true]]);
	});
});
