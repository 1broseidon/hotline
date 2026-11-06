import { expect, test } from "bun:test";
import type { BrowserInput, BrowserSnapshot } from "../src/generated/contract";
import { InputQueue, isNewer, join, keyInput, lockKind, pagePoint, scrollInput, textInputs } from "../src/remoteBrowser";

test("A point on the picture lands on the page through its CSS size, past the margin beside a letterboxed picture", () => {
	// A 1280 × 800 page in an 800 × 400 box is drawn 640 × 400, 80 in from each side.
	const box = { left: 10, top: 20, width: 800, height: 400 };
	const size = { width: 1280, height: 800 };
	expect(pagePoint(box, size, 10 + 80, 20)).toEqual({ x: 0, y: 0 });
	expect(pagePoint(box, size, 10 + 400, 20 + 200)).toEqual({ x: 640, y: 400 });
	expect(pagePoint(box, size, 10 + 720, 20 + 400)).toEqual({ x: 1280, y: 800 });
	// The margin is not the page.
	expect(pagePoint(box, size, 10 + 40, 20 + 200)).toBeNull();
	expect(pagePoint(box, { width: 0, height: 0 }, 50, 50)).toBeNull();
});

test("Keys go to the page as the desk names them, characters as text, and shortcuts stay with the window", () => {
	const press = (key: string, mods: Partial<{ shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }> = {}) =>
		keyInput({ key, shiftKey: false, ctrlKey: false, metaKey: false, ...mods });
	for (const key of ["Enter", "Tab", "Escape", "Backspace", "Delete", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown"]) {
		expect(press(key)).toEqual({ kind: "key", key });
	}
	expect(press("Tab", { shiftKey: true })).toEqual({ kind: "key", key: "Shift+Tab" });
	expect(press("a")).toEqual({ kind: "text", text: "a" });
	expect(press("A", { shiftKey: true })).toEqual({ kind: "text", text: "A" });
	expect(press(" ")).toEqual({ kind: "text", text: " " });
	expect(press("😀")).toEqual({ kind: "text", text: "😀" });
	expect(press("v", { metaKey: true })).toBeNull();
	expect(press("c", { ctrlKey: true })).toBeNull();
	expect(press("Shift", { shiftKey: true })).toBeNull();
	expect(press("F5")).toBeNull();
});

test("Text and scrolls keep within what the desk takes, and waiting ones add up", () => {
	expect(textInputs("x".repeat(25_000)).map((one) => (one.kind === "text" ? one.text.length : 0))).toEqual([10_000, 10_000, 5_000]);
	expect(scrollInput(0, 99_999, null)).toEqual({ kind: "scroll", dx: 0, dy: 20_000 });
	expect(scrollInput(-1.4, 30.6, { x: 5, y: 6 })).toEqual({ kind: "scroll", dx: -1, dy: 31, x: 5, y: 6 });
	expect(join({ kind: "text", text: "he" }, { kind: "text", text: "llo" })).toEqual({ kind: "text", text: "hello" });
	expect(join({ kind: "text", text: "x".repeat(9_999) }, { kind: "text", text: "yy" })).toBeNull();
	expect(join({ kind: "scroll", dx: 0, dy: 100, x: 1, y: 1 }, { kind: "scroll", dx: 3, dy: 50, x: 2, y: 2 })).toEqual({ kind: "scroll", dx: 3, dy: 150, x: 2, y: 2 });
	expect(join({ kind: "scroll", dx: 0, dy: 19_000 }, { kind: "scroll", dx: 0, dy: 2_000 })).toBeNull();
	expect(join({ kind: "text", text: "a" }, { kind: "key", key: "Enter" })).toBeNull();
});

test("Input goes one at a time and in order, joining what waits, and a refusal drops what waited behind it", async () => {
	const sent: BrowserInput[] = [];
	const answers: (() => void)[] = [];
	const pictures: (BrowserSnapshot | null)[] = [];
	const refusals: unknown[] = [];
	const queue = new InputQueue(
		(input) => {
			sent.push(input);
			return new Promise((resolve, reject) => answers.push(() => (input.kind === "key" && input.key === "Delete" ? reject(new Error("Take over first")) : resolve(null))));
		},
		(picture) => pictures.push(picture),
		(error) => refusals.push(error),
	);
	const settle = async () => {
		answers.shift()?.();
		await new Promise((resolve) => setTimeout(resolve, 0));
	};
	queue.push({ kind: "text", text: "h" });
	queue.push({ kind: "text", text: "i" });
	queue.push({ kind: "text", text: "!" });
	queue.push({ kind: "key", key: "Enter" });
	expect(sent).toEqual([{ kind: "text", text: "h" }]);
	await settle();
	expect(sent.at(-1)).toEqual({ kind: "text", text: "i!" });
	await settle();
	expect(sent.at(-1)).toEqual({ kind: "key", key: "Enter" });
	await settle();
	expect(pictures).toHaveLength(3);

	queue.push({ kind: "key", key: "Delete" });
	queue.push({ kind: "text", text: "lost" });
	await settle();
	expect(refusals).toHaveLength(1);
	expect(sent.at(-1)).toEqual({ kind: "key", key: "Delete" });

	queue.push({ kind: "text", text: "a" });
	queue.push({ kind: "text", text: "b" });
	queue.clear();
	await settle();
	expect(sent.map((one) => (one.kind === "text" ? one.text : one.kind))).toEqual(["h", "i!", "key", "key", "a"]);
});

test("The lock says this machine's own, a certificate let through, https, or nothing", () => {
	expect(lockKind("http://localhost:3000/")).toBe("local");
	expect(lockKind("https://192.168.1.4/", true)).toBe("local");
	expect(lockKind("https://expired.badssl.com/", true)).toBe("excepted");
	expect(lockKind("https://example.com/")).toBe("secure");
	expect(lockKind("http://example.com/")).toBe("");
	expect(lockKind("not a url")).toBe("");
});

test("An older picture never replaces a newer one, however late it answers", () => {
	const at = (when: number): BrowserSnapshot => ({ jpeg: "x", at: when, url: "", title: "" });
	expect(isNewer(undefined, at(1))).toBe(true);
	expect(isNewer(at(2), at(1))).toBe(false);
	expect(isNewer(at(2), at(2))).toBe(true);
	expect(isNewer(at(2), null)).toBe(false);
});
