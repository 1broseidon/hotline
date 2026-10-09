import { describe, expect, test } from "bun:test";
import { NAMES, suggestName } from "../src/names";

describe("the name die", () => {
	test("rolls from three hundred different names", () => {
		expect(NAMES.length).toBe(300);
		expect(new Set(NAMES).size).toBe(300);
	});

	test("never names a teammate after an AI product", () => {
		for (const product of ["Claude", "Gemini", "Copilot", "Grok", "Siri", "Alexa", "Cortana", "Bard"]) expect(NAMES).not.toContain(product);
	});

	test("never rolls the name already showing", () => {
		const first = NAMES[0]!;
		for (let i = 0; i < 50; i++) expect(suggestName(first, () => 0)).not.toBe(first);
		expect(suggestName(` ${first} `, () => 0)).not.toBe(first);
	});
});
