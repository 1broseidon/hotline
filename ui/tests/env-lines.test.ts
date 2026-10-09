import { expect, test } from "bun:test";
import { envToLines, linesToEnv } from "../src/envLines";

test("variables read as NAME=value lines and go back the same way", () => {
	const env = { API_KEY: "abc=def", ROOT: "/some path" };
	expect(envToLines(env)).toBe("API_KEY=abc=def\nROOT=/some path");
	expect(linesToEnv(envToLines(env))).toEqual({ env });
	expect(linesToEnv("\n# a comment\n  TOKEN = x  \n")).toEqual({ env: { TOKEN: "x" } });
	expect(linesToEnv("")).toEqual({ env: {} });
	expect(envToLines(undefined)).toBe("");
});

test("a line that is not a variable is named", () => {
	expect(linesToEnv("GOOD=1\njust words")).toEqual({ error: "Line 2: write each variable as NAME=value." });
	expect(linesToEnv("1BAD=x")).toEqual({ error: "Line 1: write each variable as NAME=value." });
});
