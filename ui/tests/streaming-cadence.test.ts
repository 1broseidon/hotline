import { expect, test } from "bun:test";
import { REST, step } from "../src/cadence";
import { withStreaming } from "../src/transcriptFolds";

const paragraph = "This paragraph has enough words to be a complete bubble, with room for another sentence.";

test("a long-running reply keeps stable stamps without becoming unpaced history", () => {
	const streamed = [{ messageId: "m", kind: "agent" as const, ts: 100, text: `${paragraph}\n\n${paragraph}\n\n` }];
	const blocks = withStreaming([], streamed);
	const bubbles = blocks.flatMap((block) => block.kind === "event" && block.event.kind === "agent"
		? [{ id: block.event.id, text: block.event.text, ts: block.event.ts, live: block.live === true }] : []);
	expect(bubbles.map((one) => one.ts)).toEqual([100, 100]);
	const first = step(REST, bubbles, 20_000);
	expect(first.hidden).toEqual(["m-2"]);
	expect(first.dueIn).toBeGreaterThan(0);
	const next = step(first.cadence, bubbles, 20_000 + first.dueIn!);
	expect(next.hidden).toEqual([]);
	expect([...next.cadence.shown]).toEqual(["m", "m-2"]);
	// A reconnect's written history still appears at once.
	expect(step(REST, bubbles.map((one) => ({ ...one, live: false })), 20_000).hidden).toEqual([]);
});

test("open code fences wait for closing, while thoughts remain visible", () => {
	const live = { messageId: "m", kind: "agent" as const, ts: 10, text: `${paragraph}\n\n\u0060\u0060\u0060ts\nconst x = 1;\n\n` };
	const before = withStreaming([], [live]);
	expect(before).toHaveLength(1);
	const after = withStreaming([], [{ ...live, text: `${live.text}\u0060\u0060\u0060\n\n` }, { messageId: "t", kind: "thought", ts: 11, text: "Checking" }]);
	expect(after).toHaveLength(3);
	expect(after[0]).toEqual(before[0]);
	expect(after[2]).toMatchObject({ kind: "steps", items: [{ kind: "thought", ts: 11, text: "Checking" }] });
});
