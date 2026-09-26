import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { TranscriptEvent } from "../src/generated/contract";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Transcript, deliveryLine, deliveryMissed, peerLine, superseded, turnCauseLine, stepRuns, stepsSummary } = await import("../src/components/Transcript");
const { Thread } = await import("../src/components/Thread");

const handoff = {
	kind: "handoff",
	personaId: "mack",
	name: "Mack",
	threadKey: "ada~mack",
	requestId: "request-123",
	about: "Implement the fix",
} as const;
const delivery: Extract<TranscriptEvent, { kind: "delivery" }> = {
	kind: "delivery", id: "delivery-123", ts: 1, cause: handoff,
	text: "Implement the fix and report back.", receipt: "read",
};
const pause: Extract<TranscriptEvent, { kind: "exchange_paused" }> = {
	kind: "exchange_paused", id: "pause-123", ts: 2,
	withPersonaId: "mack", withName: "Mack", exchanges: 12, status: "pending",
};

function transcript(events: TranscriptEvent[]): string {
	return renderToStaticMarkup(
		<Transcript personaId="ada" name="Ada" events={events} streaming={[]} live={false} focus={null} />,
	);
}

describe("Ask or hand off", () => {
	test("step counts belong to each run, not the whole thread", () => {
		const events: TranscriptEvent[] = [
			{ kind: "thought", id: "old-thought", ts: 1, text: "Earlier work" },
			{ kind: "turn", id: "old-turn", ts: 2, stopReason: "end_turn" },
			delivery,
			{ kind: "thought", id: "new-thought", ts: 3, text: "New work" },
			{ kind: "thought", id: "new-thought-2", ts: 4, text: "Another step" },
		];
		const runs = stepRuns(events, []);
		expect(runs.map((run) => stepsSummary(run.items))).toEqual(["1 step", "2 steps"]);
		expect(runs.map((run) => run.id)).toEqual(["old-thought", "new-thought"]);
	});

	test("a handoff is quiet and inspectable, and says when it is still queued", () => {
		expect(deliveryLine(delivery)).toEqual({ name: "Mack", said: "handed you: Implement the fix and report back." });
		expect(deliveryMissed(delivery)).toBe(false);
		const html = transcript([delivery]);
		expect(html).toContain("<button");
		expect(html).toContain("handed you: Implement the fix");
		expect(html).not.toContain("queued");
		expect(html).not.toContain("Linked with");
		const queued = transcript([{ ...delivery, receipt: "sent" }]);
		expect(queued).toContain("· queued");
		expect(turnCauseLine([{ ...delivery, receipt: "sent" }])).toBeNull();
	});

	test("a quoted line reads as words, not markdown", () => {
		expect(deliveryLine({ ...delivery, text: "`GET /receipts` is **live**" })?.said).toBe("handed you: GET /receipts is live");
	});

	test("the inspector retains sender, request and originating reply route", () => {
		const html = renderToStaticMarkup(
			<Thread open={{ key: handoff.threadKey, withName: handoff.name, handoff }}
				selfId="ada" selfName="Ada" onClose={() => {}} />,
		);
		expect(html).toContain("Handed off from Mack");
		expect(html).toContain("Mack · mack");
		expect(html).toContain("request-123");
		expect(html).toContain("Reply goes to");
		expect(html).toContain("even if they have moved on");
		expect(html).toContain("ada~mack");
	});

	test("the pending pair pause offers Keep going and Stop exchange", () => {
		const html = transcript([pause]);
		expect(html).toContain("Ada");
		expect(html).toContain("Mack");
		expect(html).toContain("12");
		expect(html).toContain("Keep going");
		expect(html).toContain("Stop exchange");
		expect(html).not.toContain("Unlink");
	});

	for (const [status, label] of [["resumed", "Exchange resumed"], ["stopped", "Exchange stopped"]] as const) {
		test(`a ${status} pause has no live decision buttons`, () => {
			const html = transcript([{ ...pause, status }]);
			expect(html).toContain(label);
			expect(html).not.toContain("Keep going");
			expect(html).not.toContain("Stop exchange");
		});
	}

	test("Answering names the sender only until the person speaks or the turn finishes", () => {
		expect(turnCauseLine([delivery])).toBe("Answering Mack");
		expect(turnCauseLine([delivery, { kind: "user", id: "user-1", ts: 2, text: "Do this instead" }])).toBeNull();
		expect(turnCauseLine([delivery, { kind: "turn", id: "turn-1", ts: 2, stopReason: "end_turn" }])).toBeNull();
	});

	test("ask replies still open as answers, not handoffs", () => {
		const answer = { ...delivery, cause: { ...handoff, kind: "peer" as const, status: "done" as const } };
		expect(deliveryLine({ ...answer, text: "pong" })).toEqual({ name: "Mack", said: "pong" });
		expect(turnCauseLine([answer])).toBe("Answering Mack");
		expect(deliveryLine({ ...answer, cause: { ...answer.cause, status: "failed" } })).toEqual({
			name: "Mack",
			said: "didn't answer · Implement the fix",
		});
	});

	const marker: Extract<TranscriptEvent, { kind: "peer" }> = {
		kind: "peer", id: "peer-1", ts: 0, threadKey: "ada~mack", withPersonaId: "mack", withName: "Mack",
		role: "caller", exchanges: 1, status: "done",
	};

	test("a thread is said once: as its answer when one came back", () => {
		const answer = { ...delivery, text: "pong", cause: { ...handoff, kind: "peer" as const, status: "done" as const } };
		expect([...superseded([marker, answer])]).toEqual(["peer-1"]);
		expect([...superseded([{ ...marker, status: "waiting" }, answer])]).toEqual([]);
		const html = transcript([marker, answer]);
		expect(html).toContain("pong");
		expect(html).not.toContain("Talked with Mack");
		expect(peerLine(marker)).toBe("Talked with Mack");
		expect(peerLine({ ...marker, status: "waiting" })).toBe("Waiting on Mack");
		expect(peerLine({ ...marker, role: "target", exchanges: 3 })).toBe("Mack asked · 3 messages");
	});

	test("your answer to a card is on the card, not a line after it", () => {
		const html = transcript([
			{ kind: "human_action", id: "h", ts: 0, actionId: "a", reason: "Which colour?", status: "done", note: "blue" },
			{ kind: "delivery", id: "d", ts: 1, text: "blue", cause: { kind: "answer", actionId: "a", status: "done", about: "Which colour?" } },
		]);
		expect(html).toContain("blue");
		expect(html).not.toContain("Picking up your answer");
		expect(html.match(/blue/g)?.length).toBe(1);
	});
});
