import { describe, expect, test } from "bun:test";
import { outputText, plainText, stepTitle } from "../src/stepText";

describe("Step titles", () => {
	test("a raw tool id reads as words", () => {
		expect(stepTitle("computer__browser ")).toBe("computer · browser");
		expect(stepTitle("ketch__search")).toBe("ketch · search");
	});

	test("a shell wrapper and a leading cd come off the command", () => {
		expect(stepTitle(`/usr/bin/zsh -lc "cd /home/george/app && git log --oneline"`)).toBe("Run git log --oneline");
		expect(stepTitle("Run cd ~/Projects/ketch && gh issue view 47 …")).toBe("Run gh issue view 47 …");
	});

	test("a home directory reads as ~, and plain titles are left alone", () => {
		expect(stepTitle("Read /home/george/notes.md")).toBe("Read ~/notes.md");
		expect(stepTitle("Read File")).toBe("Read File");
		expect(stepTitle("Run cd somewhere")).toBe("Run cd somewhere");
	});
});

describe("Step output", () => {
	test("colour codes and progress redraws are gone", () => {
		expect(plainText("\u001b[32mok\u001b[0m\r\n[0/4 built]\r[4/4 built]\n\n\n\ndone  ")).toBe("ok\n[4/4 built]\n\ndone");
	});

	test("a shell tool's JSON shows what the command printed", () => {
		const text = JSON.stringify({ status: "exited", exit_code: 2, stdout: "\u001b[1mbuilding\u001b[0m\n", stderr: "error: nope\n" });
		expect(outputText({ type: "text", text })).toBe("building\nerror: nope\nexit 2");
	});

	test("JSON that is not a shell result is shown as it is", () => {
		expect(outputText({ type: "text", text: '{"peers":[]}' })).toBe('{"peers":[]}');
	});

	test("an edit shows the lines it changed, not the whole file", () => {
		const oldText = ["a", "b", "c", "d", "e", "f", "g", "h"].join("\n");
		const newText = ["a", "b", "c", "d", "E", "f", "g", "h"].join("\n");
		expect(outputText({ type: "diff", path: "/home/george/x.ts", oldText, newText })).toBe(
			["~/x.ts", "  …", "  c", "  d", "- e", "+ E", "  f", "  g", "  …"].join("\n"),
		);
	});

	test("a new file is all additions", () => {
		expect(outputText({ type: "diff", path: "/tmp/n.md", oldText: null, newText: "one\ntwo" })).toBe("/tmp/n.md\n+ one\n+ two");
	});
});
