import { describe, expect, test } from "bun:test";
import { sameWork } from "../src/components/Work";
import { sideCommand } from "../src/components/Conversation";

describe("pressing what opened a card", () => {
	test("is the same work only for the same turn", () => {
		expect(sameWork({ personaId: "p", blockId: null }, { personaId: "p", blockId: null })).toBe(true);
		expect(sameWork({ personaId: "p", blockId: "b1" }, { personaId: "p", blockId: "b2" })).toBe(false);
		expect(sameWork({ personaId: "p", blockId: "b1" }, { personaId: "p", blockId: null })).toBe(false);
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
