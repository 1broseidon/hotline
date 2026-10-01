import { beforeEach, describe, expect, test } from "bun:test";
import type { FileChunk } from "../src/generated/contract";
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { avatarUrl, forgetAvatars } = await import("../src/avatars");

const chunk = (data: string, next?: number): FileChunk => ({ name: "a.png", mimeType: "image/png", size: 6, offset: 0, data, ...(next !== undefined ? { next } : {}) });

describe("A teammate's picture", () => {
	beforeEach(forgetAvatars);

	test("is read a part at a time and made once per hash", async () => {
		const offsets: number[] = [];
		const read = async (_personaId: string, _hash: string, offset: number) => {
			offsets.push(offset);
			return offset === 0 ? chunk(btoa("abc"), 3) : chunk(btoa("def"));
		};
		const first = await avatarUrl("ada", "h1", read);
		const again = await avatarUrl("ada", "h1", read);
		expect(again).toBe(first);
		expect(offsets).toEqual([0, 3]);
		expect(await (await fetch(first)).text()).toBe("abcdef");
	});

	test("that could not be read is asked for again next time", async () => {
		let calls = 0;
		const read = async () => {
			calls++;
			if (calls === 1) throw new Error("That teammate has no such picture.");
			return chunk(btoa("abc"));
		};
		await expect(avatarUrl("ada", "h2", read)).rejects.toThrow("no such picture");
		await avatarUrl("ada", "h2", read);
		expect(calls).toBe(2);
	});
});
