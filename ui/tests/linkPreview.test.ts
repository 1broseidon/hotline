import { describe, expect, test } from "bun:test";
import { fetchable, onlyLink, previewLink } from "../src/linkPreview";

describe("the link a message gets a card for", () => {
	test("is the first bare https address", () => {
		expect(previewLink("see https://github.com/1broseidon/hotline/pull/165. and https://example.com")).toBe(
			"https://github.com/1broseidon/hotline/pull/165",
		);
		expect(previewLink("<https://example.com/a>")).toBe("https://example.com/a");
	});

	test("is never one given words of its own, one in code, or one on this network", () => {
		expect(previewLink("read [the PR](https://github.com/x/y/pull/1)")).toBeNull();
		expect(previewLink("run `curl https://example.com`")).toBeNull();
		expect(previewLink("```\nhttps://example.com\n```")).toBeNull();
		expect(previewLink("http://example.com")).toBeNull();
		expect(previewLink("https://192.168.1.4/admin")).toBeNull();
		expect(fetchable("https://nas.local")).toBe(false);
	});

	test("a message that is only the link is told apart", () => {
		expect(onlyLink(" https://example.com/ ", "https://example.com")).toBe(true);
		expect(onlyLink("look https://example.com", "https://example.com")).toBe(false);
	});
});
