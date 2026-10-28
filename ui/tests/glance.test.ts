import { describe, expect, test } from "bun:test";
import { glance } from "../src/glance";
import type { Step } from "../src/scheduledRuns";

let at = 0;
const tool = (title: string, toolKind?: string, more: Partial<Extract<Step, { kind: "tool" }>> = {}): Step => ({
	kind: "tool",
	id: `s${++at}`,
	ts: at,
	toolCallId: `c${at}`,
	title,
	...(toolKind !== undefined ? { toolKind } : {}),
	status: "completed",
	...more,
});
const thought = (text: string): Step => ({ kind: "thought", id: `t${++at}`, ts: at, text });
const texts = (steps: Step[], running = false) => glance(steps, running).map((line) => line.text);

describe("a turn at a glance", () => {
	test("says what was done in a few words, never the command itself", () => {
		expect(texts([tool("python3 -I - <<'E' import json d=json.load(open(\"x\"))", "execute")])).toEqual(["Ran a Python script"]);
		expect(texts([tool("git status --short", "execute")])).toEqual(["Ran git status"]);
		expect(texts([tool("zsh -lc 'cd /tmp && cargo test -p core'", "execute")])).toEqual(["Ran cargo test"]);
		expect(texts([tool("read .agents/skills/hotline-room/SKILL.md", "read")])).toEqual(["Read SKILL.md"]);
		expect(texts([tool("web_fetch: https://github.com/kyutai-labs/moshi/", "fetch")])).toEqual(["Read a web page"]);
		expect(texts([tool("mcp__claude_ai_Prism__linear__list_issues", "other")])).toEqual(["Used Linear"]);
		expect(texts([tool("mcp__brainfile__get_task", "other")])).toEqual(["Used the board"]);
		expect(texts([tool("computer__browser", "computer__browser")])).toEqual(["Used the browser"]);
		expect(texts([tool("ToolSearch", "other")])).toEqual(["Looked up its tools"]);
	});

	test("steps of one kind in a row are one line with their count, and its id is the first of them", () => {
		const steps = [tool("git status", "execute"), tool("ls src", "execute"), tool("read a.ts", "read"), tool("read b.ts", "read"), tool("read c.ts", "read")];
		const lines = glance(steps, false);
		expect(lines.map((line) => line.text)).toEqual(["Ran 2 commands", "Read 3 files"]);
		expect(lines[1]!.ids).toEqual([steps[2]!.id, steps[3]!.id, steps[4]!.id]);
		expect(lines[1]!.id).toBe(steps[2]!.id);
		expect(texts([tool("mcp__brainfile__get_task", "other"), tool("mcp__brainfile__get_task", "other")])).toEqual(["Used the board twice"]);
	});

	test("while the turn runs its last line says what is happening now, in the newest step's words", () => {
		const lines = glance([thought("hmm"), tool("read a.ts", "read"), tool("git log", "execute"), tool("cargo test", "execute")], true);
		expect(lines.map((line) => line.text)).toEqual(["Thought it through", "Read a.ts", "Running cargo test"]);
		expect(lines.map((line) => line.live)).toEqual([false, false, true]);
	});

	test("a failed step marks its line", () => {
		expect(glance([tool("make", "execute", { status: "failed" })], false)[0]!.failed).toBe(true);
	});
});
