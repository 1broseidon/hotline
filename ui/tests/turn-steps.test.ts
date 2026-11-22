import { describe, expect, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";
import { keepBlocks, stepRuns, toBlocks, withStreaming } from "../src/transcriptFolds";
import { turnTook, workLine } from "../src/turnFacts";

const user = (id: string, ts: number) => ({ kind: "user", id, ts, text: "plan the release" } as const);
const agent = (id: string, ts: number) => ({ kind: "agent", id, ts, text: `said ${id}` } as const);
const tool = (id: string, ts: number) => ({ kind: "tool", id, ts, toolCallId: id, title: `step ${id}`, status: "completed" } as const);
const thought = (id: string, ts: number) => ({ kind: "thought", id, ts, text: "hmm" } as const);
const turn = (id: string, ts: number) => ({ kind: "turn", id, ts, stopReason: "end_turn" } as const);
const chapter = (id: string, ts: number) => ({ kind: "chapter", id, ts, backendId: "agent" } as const);

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

	test("a phone reply with one recorded thought uses the time from the prompt to the turn end", () => {
		const end = turn("end", 1795222815958);
		const beforeEnd: TranscriptEvent[] = [
			user("previous-prompt", 1795222800000),
			agent("previous-reply", 1795222809990),
			turn("previous-end", 1795222810000),
			chapter("chapter", 1795222810621),
			{ ...user("phone-prompt", 1795222810651), receipt: "read", client: "phone" },
			thought("thought", 1795222814952),
			agent("reply", 1795222815955),
		];
		const events = [...beforeEnd, end];
		const blocks = toBlocks(events);
		const steps = blocks.find((block) => block.kind === "steps");
		expect(steps).toMatchObject({ id: "thought", ts: 1795222815955, elapsed: 5307 });
		const run = stepRuns(events, [])[0]!;
		expect(run.elapsed).toBe(5307);
		expect(workLine(run.items, run.elapsed).took).toBe("5.3s");

		// The terminal line can change the time without changing the steps or the reply's stamp.
		const changed = keepBlocks(blocks, toBlocks([...beforeEnd, { ...end, ts: end.ts + 1000 }]));
		expect(changed.find((block) => block.kind === "steps")?.elapsed).toBe(6307);
	});

	test("a positive path span wins, including a tool's end", () => {
		const step = { ...tool("tool", 2000), endedTs: 3500 };
		const [run] = stepRuns([user("prompt", 1000), step, agent("reply", 4000), turn("end", 5000)], []);
		expect(run?.elapsed).toBeUndefined();
		expect(turnTook([step], 4000)).toBe(1500);
	});

	test("a prompt does not lend its time across a chapter, a turn end, or a new unread message", () => {
		const prompt = user("prompt", 1);
		const boundaries: TranscriptEvent[] = [chapter("chapter", 2), turn("older-end", 2), { ...user("queued", 2), receipt: "sent" }];
		for (const boundary of boundaries) {
			const [run] = stepRuns([prompt, boundary, thought("thought", 3), agent("reply", 4), turn("end", 5)], []);
			expect(run?.elapsed).toBeUndefined();
		}
	});

	test("quiet, unprompted, and unfinished turns do not infer a duration", () => {
		const prompt = user("prompt", 1);
		const reply = [thought("thought", 2), agent("reply", 3)];
		const end = turn("end", 4);
		const [quiet] = stepRuns([prompt, ...reply, { ...end, quiet: true }], []);
		const [unprompted] = stepRuns([...reply, end], []);
		const [live] = stepRuns([prompt, ...reply], []);
		expect(quiet?.elapsed).toBeUndefined();
		expect(unprompted?.elapsed).toBeUndefined();
		expect(live?.open).toBe(true);
		expect(live?.elapsed).toBeUndefined();
	});

	test("inline paths keep their timing and an entirely pathless reply gains no block", () => {
		const prompt = user("prompt", 1);
		const reply = agent("reply", 2);
		const end = turn("end", 4);
		const inline = toBlocks([prompt, reply, thought("thought", 3), end], true);
		expect(inline.find((block) => block.kind === "steps")?.elapsed).toBeUndefined();
		expect(shape(toBlocks([prompt, reply, end]))).toEqual(["prompt", "reply", "end"]);
	});

	test("a run says whether its turn is still open, so a turn just begun is never taken for more of the last", () => {
		const ended = stepRuns([user("u1", 1), tool("s1", 2), agent("a1", 3), turn("end", 4), user("u2", 5)], []);
		expect(ended.map((run) => [run.id, run.open === true])).toEqual([["s1", false]]);
		const begun = stepRuns([user("u1", 1), tool("s1", 2), agent("a1", 3), turn("end", 4), user("u2", 5)], [{ messageId: "t2", kind: "thought", ts: 6, text: "thinking" }]);
		expect(begun.map((run) => [run.id, run.open === true])).toEqual([["s1", false], ["t2", true]]);
	});
});
