import { describe, expect, test } from "bun:test";
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { sshTarget } = await import("../src/components/AddDesk");

describe("an SSH target", () => {
	test("is user@host or a config host", () => {
		expect(sshTarget("ada@studio.example")).toBe(true);
		expect(sshTarget("studio")).toBe(true);
	});

	test("never starts with a dash, has no spaces and is not empty, as the shell requires", () => {
		expect(sshTarget("-oProxyCommand=evil")).toBe(false);
		expect(sshTarget("-p")).toBe(false);
		expect(sshTarget("ada@studio example")).toBe(false);
		expect(sshTarget("")).toBe(false);
	});
});
