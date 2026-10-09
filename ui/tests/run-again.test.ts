import { expect, test } from "bun:test";
import { errorDetails } from "../src/components/ErrorCard";
import { failedLast } from "../src/retryTurn";
import type { TranscriptEvent } from "../src/generated/contract";

const user = (id: string): TranscriptEvent => ({ kind: "user", id, ts: 1, text: "did the crane jam?" }) as TranscriptEvent;
const notice = (id: string): TranscriptEvent => ({ kind: "notice", id, ts: 2, level: "error", text: "Turn failed" }) as TranscriptEvent;
const turn = (id: string, stopReason: string): TranscriptEvent => ({ kind: "turn", id, ts: 3, stopReason }) as TranscriptEvent;

test("only the failure of the latest turn, with nothing said since, can run again", () => {
	expect(failedLast([user("u1"), notice("n1"), turn("t1", "failed")], "n1")).toBe(true);
	// Said again afterwards, or answered since: not this notice's to retry.
	expect(failedLast([user("u1"), notice("n1"), turn("t1", "failed"), user("u2")], "n1")).toBe(false);
	expect(failedLast([user("u1"), notice("n1"), turn("t1", "failed"), turn("t2", "end_turn")], "n1")).toBe(false);
	// A notice whose turn ended some other way, or one not on the tape.
	expect(failedLast([user("u1"), notice("n1"), turn("t1", "cancelled")], "n1")).toBe(false);
	expect(failedLast([user("u1")], "missing")).toBe(false);
});

test("a failure carries its kind, which decides whether another try is offered", () => {
	const text = `Provider connection interrupted: The request could not complete over the network.\n\n${JSON.stringify({
		hotlineFailure: { kind: "transport", title: "Provider connection interrupted", summary: "The request could not complete over the network.", details: "dns error" },
	})}`;
	expect(errorDetails(text).kind).toBe("transport");
	expect(errorDetails("plain words").kind).toBeUndefined();
});
