import { afterEach, beforeEach, expect, test } from "bun:test";
import { storedNumbers, storeNumbers } from "../src/storage";
import { installDom } from "./dom";

let teardown: () => Promise<void>;
beforeEach(() => {
	({ teardown } = installDom());
});
afterEach(async () => {
	await teardown();
});

test("a record of numbers comes back as it was stored", () => {
	storeNumbers("hotline.test.seen", { ada: 3, kit: 1_800_000_000_000 });
	expect(storedNumbers("hotline.test.seen")).toEqual({ ada: 3, kit: 1_800_000_000_000 });
	expect(storedNumbers("hotline.test.never")).toEqual({});
});

test("a corrupt value reads as empty, and only finite numbers survive a hand edit", () => {
	localStorage.setItem("hotline.test.seen", "{not json");
	expect(storedNumbers("hotline.test.seen")).toEqual({});
	localStorage.setItem("hotline.test.seen", "[1, 2]");
	expect(storedNumbers("hotline.test.seen")).toEqual({});
	localStorage.setItem("hotline.test.seen", JSON.stringify({ ada: 3, kit: "4", bo: null }));
	expect(storedNumbers("hotline.test.seen")).toEqual({ ada: 3 });
});
