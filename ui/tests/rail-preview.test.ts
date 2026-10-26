import { expect, test } from "bun:test";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { oneLine } = await import("../src/components/Rail");

test("a table in a reply never reaches the row; the prose around it does", () => {
	const reply = "| | Hotline | Grok Bot |\n|---|---|---|\n| Any model | Yes | No |\n\nHotline is the only one you can run on your own machine.";
	expect(oneLine(reply)).toBe("Hotline is the only one you can run on your own machine.");
});

test("a reply that is only a table reads as one", () => {
	expect(oneLine("| a | b |\n|---|---|\n| 1 | 2 |")).toBe("Table");
});

test("other markdown is still stripped to one line", () => {
	expect(oneLine("**Railway** asked\n\n- rotate `keys`")).toBe("Railway asked rotate keys");
});
