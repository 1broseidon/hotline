import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { TranscriptEvent } from "../src/generated/contract";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Transcript, deliveryLine, deliveryMissed, turnCauseLine } = await import("../src/components/Transcript");
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
	test("a handoff is quiet, inspectable and has read receipts", () => {
		expect(deliveryLine(delivery)).toBe("Handed off from Mack · Implement the fix");
		expect(deliveryMissed(delivery)).toBe(false);
		const html = transcript([delivery]);
		expect(html).toContain("<button");
		expect(html).toContain("Handed off from Mack");
		expect(html).toContain('aria-label="Read"');
		expect(html).not.toContain("Linked with");
	});

	test("a queued handoff has a sent receipt, not a read receipt", () => {
		const html = transcript([{ ...delivery, receipt: "sent" }]);
		expect(html).toContain('aria-label="Sent"');
		expect(html).not.toContain('aria-label="Read"');
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
		expect(deliveryLine(answer)).toBe("Mack answered · Implement the fix");
		expect(turnCauseLine([answer])).toBe("Answering Mack");
		expect(deliveryLine({ ...answer, cause: { ...answer.cause, status: "failed" } })).toBe("Mack didn't answer · Implement the fix");
	});
});
