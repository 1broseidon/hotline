import { describe, expect, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { foldReactions, reactionQuote } = await import("../src/components/Transcript");

const agent = (id: string, text: string): TranscriptEvent => ({ kind: "agent", id, ts: 1, text });
const user = (id: string, text: string, replyTo?: string): TranscriptEvent => ({
	kind: "user",
	id,
	ts: 2,
	text,
	...(replyTo !== undefined ? { replyTo } : {}),
});

describe("Reactions sent as lines", () => {
	test("this window's reaction folds onto the line it answers", () => {
		const events = [agent("a1", "Shall I merge it?\nCI is green."), user("u1", `${reactionQuote("Shall I merge it?\nCI is green.")}\n\n👍`, "a1")];
		const { lines, on } = foldReactions(events);
		expect([...lines]).toEqual(["u1"]);
		expect(on.get("a1")).toEqual(["👍"]);
	});

	test("an older phone reaction with no replyTo is matched by its quote", () => {
		const events = [agent("a1", "Done."), agent("a2", "Shipped it."), user("u1", "> Done.\n\n❤️")];
		expect(foldReactions(events).on.get("a1")).toEqual(["❤️"]);
	});

	test("words, or an emoji with no quote, stay messages", () => {
		const events = [agent("a1", "Done."), user("u1", "> Done.\n\nthanks", "a1"), user("u2", "👍", "a1")];
		const { lines, on } = foldReactions(events);
		expect(lines.size).toBe(0);
		expect(on.size).toBe(0);
	});

	test("the quote folds whitespace and stops at 140 characters, as the phone's does", () => {
		expect(reactionQuote("  one\n\n two  ")).toBe("> one two");
		expect(reactionQuote("x".repeat(200))).toBe(`> ${"x".repeat(140)}`);
	});
});
