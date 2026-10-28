import { describe, expect, test } from "bun:test";
import { glance } from "../src/glance";
import type { Step } from "../src/scheduledRuns";

let at = 0;
type Said = { key: string; one: string; many: string; doing: string };
const tool = (title: string, said?: Said, status: "completed" | "failed" = "completed"): Step => ({
	kind: "tool",
	id: `s${++at}`,
	ts: at,
	toolCallId: `c${at}`,
	title,
	status,
	...(said !== undefined ? { said } : {}),
});
const thought = (text: string): Step => ({ kind: "thought", id: `t${++at}`, ts: at, text });
const ran = (program: string): Said => ({ key: "execute", one: `Ran ${program}`, many: "Ran {n} commands", doing: `Running ${program}` });
const read = (file: string): Said => ({ key: "read", one: `Read ${file}`, many: "Read {n} files", doing: "Reading" });

describe("a turn at a glance", () => {
	test("each step is said in the core's words, never the command itself", () => {
		const lines = glance([tool("python3 -I - <<'E' import json", ran("a Python script"))], false);
		expect(lines.map((line) => line.text)).toEqual(["Ran a Python script"]);
	});

	test("steps of one kind in a row are one line with their count, and its id is the first of them", () => {
		const steps = [tool("git status", ran("git status")), tool("ls src", ran("ls")), tool("read a.ts", read("a.ts")), tool("read b.ts", read("b.ts")), tool("read c.ts", read("c.ts"))];
		const lines = glance(steps, false);
		expect(lines.map((line) => line.text)).toEqual(["Ran 2 commands", "Read 3 files"]);
		expect(lines[1]!.ids).toEqual([steps[2]!.id, steps[3]!.id, steps[4]!.id]);
		expect(lines[1]!.id).toBe(steps[2]!.id);
	});

	test("while the turn runs its last line says what is happening now, in the newest step's words", () => {
		const lines = glance([thought("hmm"), tool("read a.ts", read("a.ts")), tool("git log", ran("git log")), tool("cargo test", ran("cargo test"))], true);
		expect(lines.map((line) => line.text)).toEqual(["Thought it through", "Read a.ts", "Running cargo test"]);
		expect(lines.map((line) => line.live)).toEqual([false, false, true]);
	});

	test("a step written before the core said it shows a few words of its title", () => {
		expect(glance([tool("Search mail since Monday morning early")], false)[0]!.text).toBe("Search mail since Monday");
	});

	test("a failed step marks its line", () => {
		expect(glance([tool("make", ran("make"), "failed")], false)[0]!.failed).toBe(true);
	});
});
