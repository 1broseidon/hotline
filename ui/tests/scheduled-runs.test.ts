import { describe, expect, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";
import { type Block, groupScheduled } from "../src/scheduledRuns";

const run = (id: string, jobId: string, ts = 1): Block => ({
	kind: "event",
	event: {
		kind: "user",
		id,
		ts,
		text: `prompt ${id}`,
		scheduled: { jobId, kind: "recurring", name: `job ${jobId}`, operatorCreated: true },
	} as TranscriptEvent as Extract<TranscriptEvent, { kind: "user" }>,
});
const event = (kind: string, id: string, extra: object = {}): Block =>
	({ kind: "event", event: { kind, id, ts: 1, text: "x", ...extra } }) as unknown as Block;
const shape = (blocks: Block[]) =>
	blocks.map((block) => (block.kind === "scheduled" ? `${block.id}x${block.runs.length}` : block.kind === "event" ? block.event.id : block.id));

describe("grouping scheduled runs", () => {
	test("a single run stays a line of its own", () => {
		expect(shape(groupScheduled([run("s1", "j")]))).toEqual(["s1"]);
	});

	test("consecutive runs of one job fold into one block at the newest run, oldest first", () => {
		const out = groupScheduled([run("s1", "j"), run("s2", "j"), run("s3", "j")]);
		expect(shape(out)).toEqual(["s3x3"]);
		const group = out[0]!;
		expect(group.kind === "scheduled" && group.runs.map((one) => one.id)).toEqual(["s1", "s2", "s3"]);
	});

	test("anything drawn between breaks the group", () => {
		const breakers = [
			event("agent", "a"),
			event("user", "u"),
			event("notice", "n", { level: "info" }),
			event("delivery", "d"),
			event("chapter", "c"),
			event("permission", "p"),
			{ kind: "steps", id: "w", ts: 1, items: [] } as Block,
			run("other", "k"),
		];
		for (const breaker of breakers) {
			const out = shape(groupScheduled([run("s1", "j"), breaker, run("s2", "j")]));
			expect(out).toHaveLength(3);
			expect(out).not.toContain("s2x2");
		}
	});

	test("what is not drawn does not break it, and stays where it was", () => {
		const out = groupScheduled([run("s1", "j"), event("turn", "t1", { stopReason: "end_turn" }), run("s2", "j"), event("turn", "t2", { stopReason: "end_turn" })]);
		expect(shape(out)).toEqual(["t1", "s2x2", "t2"]);
	});

	test("a turn that stopped for another reason is drawn and breaks it", () => {
		const out = groupScheduled([run("s1", "j"), event("turn", "t1", { stopReason: "max_tokens" }), run("s2", "j")]);
		expect(shape(out)).toEqual(["s1", "t1", "s2"]);
	});

	test("groups of two jobs, and a run left alone after a break, are each their own", () => {
		const out = groupScheduled([run("a1", "a"), run("a2", "a"), run("b1", "b"), run("b2", "b"), event("agent", "m"), run("b3", "b")]);
		expect(shape(out)).toEqual(["a2x2", "b2x2", "m", "b3"]);
	});
});
