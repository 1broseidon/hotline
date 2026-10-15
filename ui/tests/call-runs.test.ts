import { describe, expect, test } from "bun:test";
import { type Block, groupCalls } from "../src/scheduledRuns";

const call = (id: string, state: "closed" | "open" = "closed", minutes = 0): Block =>
	({
		kind: "event",
		event: { kind: "link", id, ts: 1_000, at: 1_000 + minutes * 60_000, thread: id, threadKind: "call", title: "Call", state, outcome: "Hung up" },
	}) as unknown as Block;
const event = (kind: string, id: string, extra: object = {}): Block =>
	({ kind: "event", event: { kind, id, ts: 1, text: "x", ...extra } }) as unknown as Block;
const shape = (blocks: Block[]) =>
	blocks.map((block) => (block.kind === "calls" ? `${block.id}x${block.calls.length}` : block.kind === "event" ? block.event.id : block.id));

describe("grouping calls one after another", () => {
	test("a single call stays a line of its own", () => {
		expect(shape(groupCalls([call("c1")]))).toEqual(["c1"]);
	});

	test("finished calls in a row fold into one block at the newest, oldest first", () => {
		const out = groupCalls([call("c1"), call("c2", "closed", 2), call("c3")]);
		expect(shape(out)).toEqual(["c3x3"]);
		const group = out[0]!;
		expect(group.kind === "calls" && group.calls.map((one) => one.id)).toEqual(["c1", "c2", "c3"]);
	});

	test("a call still going, or anything said between, keeps lines apart", () => {
		expect(shape(groupCalls([call("c1"), call("c2"), call("live", "open")]))).toEqual(["c2x2", "live"]);
		expect(shape(groupCalls([call("c1"), event("agent", "m"), call("c2")]))).toEqual(["c1", "m", "c2"]);
		// A side thread's line is a link too, but not a call.
		const side = { kind: "event", event: { kind: "link", id: "s", ts: 1, thread: "s", threadKind: "side", title: "Work", state: "closed" } } as unknown as Block;
		expect(shape(groupCalls([call("c1"), side, call("c2")]))).toEqual(["c1", "s", "c2"]);
	});

	test("a turn that draws nothing does not come between two calls", () => {
		const out = groupCalls([call("c1"), event("turn", "t1", { stopReason: "end_turn" }), call("c2")]);
		expect(shape(out)).toEqual(["t1", "c2x2"]);
	});
});
