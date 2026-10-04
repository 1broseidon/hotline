import { describe, expect, test } from "bun:test";
import { type CallEvent, callLine } from "../src/components/Transcript";

const call = (over: Partial<CallEvent>): CallEvent => ({
	kind: "call",
	id: "link:call:c1",
	ts: 0,
	callId: "c1",
	title: "Call",
	status: "ended",
	...over,
});

describe("a call's line in the conversation", () => {
	test("reads as in progress while it is live", () => {
		expect(callLine(call({ status: "live" }))).toBe("Call · in progress");
	});
	test("says how long it lasted and how it ended", () => {
		expect(callLine(call({ durationMs: 240_000, outcome: "Hung up" }))).toBe("Call · 4 min · Hung up");
	});
	test("a short call is under a minute, and no outcome is left out", () => {
		expect(callLine(call({ durationMs: 12_000 }))).toBe("Call · under a minute");
	});
});
