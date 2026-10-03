import { describe, expect, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";
import { runPieces, sameWork, sidePieces } from "../src/components/Work";
import { sideCommand } from "../src/components/Conversation";
import { sideLine } from "../src/components/Transcript";

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

describe("a side thread in the work card", () => {
	test("is the same work only for the same side thread", () => {
		const side = (sideId: string) => ({ personaId: "p", sideId, title: "t" });
		expect(sameWork(side("s1"), side("s1"))).toBe(true);
		expect(sameWork(side("s1"), side("s2"))).toBe(false);
		expect(sameWork(side("s1"), { personaId: "p", runId: "s1", title: "t" })).toBe(false);
		expect(sameWork({ personaId: "p", blockId: null }, side("s1"))).toBe(false);
	});

	test("shows what the person said, steps between words, cards, and what is still arriving", () => {
		const pieces = sidePieces(
			[
				line("side", "side:s1", { status: "live" }),
				line("user", "u1"),
				line("agent", "a1"),
				line("tool", "c1", { title: "read", status: "completed" }),
				line("permission", "perm:r1", { title: "Run it?", options: [], requestId: "r1" }),
				line("agent", "a2"),
				line("turn", "t1", { stopReason: "end_turn" }),
				line("user", "u2"),
			],
			[
				{ messageId: "m9", kind: "agent", text: "Working on" },
				{ messageId: "m8", kind: "thought", text: "hmm" },
			],
		);
		expect(pieces.map((piece) => `${piece.kind}:${piece.id}`)).toEqual([
			"person:u1",
			"said:a1",
			"steps:c1",
			"permission:perm:r1",
			"said:a2",
			"person:u2",
			"said:live:m9",
		]);
	});
});

describe("/side in the composer", () => {
	test("is a command with a task, or without one", () => {
		expect(sideCommand("/side fix the CI badge")).toEqual({ task: "fix the CI badge" });
		expect(sideCommand("  /SIDE   fix it\nand the docs ")).toEqual({ task: "fix it\nand the docs" });
		expect(sideCommand("/side")).toEqual({ task: "" });
		expect(sideCommand("/side ")).toEqual({ task: "" });
	});

	test("is not a command in the middle of words, or a longer word", () => {
		expect(sideCommand("please /side this")).toBeNull();
		expect(sideCommand("/sidebar is broken")).toBeNull();
		expect(sideCommand("hello")).toBeNull();
	});
});

describe("a side thread's line in the conversation", () => {
	const marker = (extra: object) => ({ kind: "side", id: "side:s1", ts: 1, sideId: "s1", personaId: "p", title: "Fix the CI badge", ...extra }) as Parameters<typeof sideLine>[0];

	test("says it started while it runs", () => {
		expect(sideLine(marker({ status: "live" })).text).toBe("Started a side thread · Fix the CI badge");
	});

	test("becomes its title and a one-line result once archived, and says when nobody ended it", () => {
		expect(sideLine(marker({ status: "archived", archivedBy: "agent", result: "Badge is green." })).text).toBe("Side thread · Fix the CI badge · Badge is green.");
		expect(sideLine(marker({ status: "archived", archivedBy: "stopped", result: "Half done." })).text).toBe("Side thread · Fix the CI badge · Half done. · stopped");
		expect(sideLine(marker({ status: "archived", archivedBy: "person" })).text).toBe("Side thread · Fix the CI badge · archived");
	});

	test("says it is parked while its agent is let go of and the thread is open", () => {
		expect(sideLine(marker({ status: "parked" })).text).toBe("Side thread · Fix the CI badge · parked");
	});
});
