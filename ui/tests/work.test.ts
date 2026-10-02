import { describe, expect, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";
import { runPieces, sameWork } from "../src/components/Work";

const line = (kind: string, id: string, extra: object = {}) => ({ kind, id, ts: 1, text: `${kind} ${id}`, ...extra }) as unknown as TranscriptEvent;

describe("a subagent's run in the work card", () => {
	test("steps gather between what it said, and its notices stay", () => {
		const pieces = runPieces([
			line("agent", "a1"),
			line("thought", "t1"),
			line("tool", "c1", { title: "read", status: "completed" }),
			line("agent", "a2"),
			line("notice", "n1", { level: "error" }),
			line("turn", "u1", { stopReason: "end_turn" }),
		]);
		expect(pieces.map((piece) => (piece.kind === "steps" ? `steps:${piece.items.map((one) => one.id).join(",")}` : `${piece.kind}:${piece.id}`))).toEqual([
			"said:a1",
			"steps:t1,c1",
			"said:a2",
			"notice:n1",
		]);
	});

	test("an empty line says nothing", () => {
		expect(runPieces([line("agent", "a1", { text: "  " })])).toEqual([]);
	});
});

describe("pressing what opened a card", () => {
	test("is the same work only for the same turn or the same run", () => {
		expect(sameWork({ personaId: "p", blockId: null }, { personaId: "p", blockId: null })).toBe(true);
		expect(sameWork({ personaId: "p", blockId: "b1" }, { personaId: "p", blockId: "b2" })).toBe(false);
		expect(sameWork({ personaId: "p", runId: "r1", title: "x" }, { personaId: "p", runId: "r1", title: "y" })).toBe(true);
		expect(sameWork({ personaId: "p", runId: "r1", title: "x" }, { personaId: "p", blockId: null })).toBe(false);
		expect(sameWork({ personaId: "p", blockId: null }, { personaId: "p", runId: "r1", title: "x" })).toBe(false);
	});
});
