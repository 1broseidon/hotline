import { describe, expect, test } from "bun:test";
import { highlight } from "../src/highlight";

const kinds = (code: string, language: string) =>
	highlight(code, language).map((line) => line.filter((run) => run.kind !== undefined).map((run) => `${run.kind}:${run.text}`));

describe("code's colour", () => {
	test("marks keywords, strings, numbers, comments and a called function", () => {
		expect(kinds('from x import y\nclient.create("docs", size=384)  # MiniLM', "python")).toEqual([
			["keyword:from", "keyword:import"],
			["function:create", 'string:"docs"', "number:384", "comment:# MiniLM"],
		]);
	});

	test("keeps every line, empty ones too, and the text exactly as written", () => {
		const code = "let a = 1;\n\n/* two\n   lines */\nconst b = `x`;";
		const lines = highlight(code, "ts");
		expect(lines.length).toBe(5);
		expect(lines.map((line) => line.map((run) => run.text).join("")).join("\n")).toBe(code);
		expect(lines[2]?.[0]).toEqual({ text: "/* two", kind: "comment" });
		expect(lines[3]?.[0]).toEqual({ text: "   lines */", kind: "comment" });
	});

	test("does not read Rust's lifetimes or a shell's $# as strings and comments", () => {
		expect(kinds("fn f<'a>(x: &'a str) -> char { 'z' }", "rust")[0]).toEqual(["keyword:fn", "string:'z'"]);
		expect(kinds("echo $# # count", "bash")[0]).toEqual(["comment:# count"]);
	});

	test("leaves a language it does not know plain", () => {
		expect(highlight("if x then y", "brainfudge")).toEqual([[{ text: "if x then y" }]]);
		expect(highlight("a\n\nb", undefined)).toEqual([[{ text: "a" }], [], [{ text: "b" }]]);
	});

	test("reads SQL's keywords in any case", () => {
		expect(kinds("SELECT name FROM t -- all", "sql")[0]).toEqual(["keyword:SELECT", "keyword:FROM", "comment:-- all"]);
	});
});
