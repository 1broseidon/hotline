import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { TranscriptEvent } from "../src/generated/contract";

// task-472: a teammate past its hourly limit on quiet turns is paused with
// the card a pair of teammates gets, for one teammate.

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }) } });
const { Transcript } = await import("../src/components/Transcript");
const { askLine, openAsks } = await import("../src/components/transcript/AskCard");

const paused: Extract<TranscriptEvent, { kind: "quiet_paused" }> = { kind: "quiet_paused", id: "quiet-paused:1", ts: 2, turns: 20, status: "pending" };

function transcript(events: TranscriptEvent[]): string {
	return renderToStaticMarkup(<Transcript personaId="huck" name="Huck" events={events} streaming={[]} live={false} focus={null} />);
}

describe("the hourly limit's card", () => {
	test("says which teammate and how many quiet turns in the hour, with Keep going and Stop", () => {
		const html = transcript([paused]);
		expect(html).toContain(">Paused<");
		expect(html).toContain("Huck paused after 20 quiet turns in an hour without you.");
		expect(html).toContain(">Keep going<");
		expect(html).toContain(">Stop<");
	});

	for (const [status, label] of [["resumed", "Quiet turns resumed"], ["stopped", "Stopped. Write to Huck to start again."]] as const) {
		test(`a ${status} one is a quiet line with no keys`, () => {
			const html = transcript([{ ...paused, status }]);
			expect(html).toContain(label);
			expect(html).not.toContain("Keep going");
			expect(html).not.toContain(">Stop<");
		});
	}

	test("while it waits it is one of the cards in Needs you, with its line", () => {
		expect(openAsks([paused]).map((ask) => ask.id)).toEqual(["quiet-paused:1"]);
		expect(askLine(openAsks([paused])[0]!)).toBe("Paused after 20 quiet turns");
		expect(openAsks([{ ...paused, status: "stopped" }])).toEqual([]);
		expect(openAsks([{ ...paused, status: "resumed" }])).toEqual([]);
	});
});
