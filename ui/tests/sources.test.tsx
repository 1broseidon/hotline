import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { Step } from "../src/scheduledRuns";
import type { ToolStep } from "../src/turnFacts";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Trail, plainTitle, ranOf } = await import("../src/components/transcript/Trail");

const search = (pages: number): Step => ({
	kind: "tool",
	id: "s1",
	ts: 1,
	toolCallId: "c1",
	title: "web_search coding harnesses",
	status: "completed",
	sources: Array.from({ length: pages }, (_, at) => ({ url: `https://site${at}.example/page`, title: `Page ${at}`, found: true })),
});

describe("the pane's sources", () => {
	test("a few pages are all cards; many show the first four and how many more", () => {
		const few = renderToStaticMarkup(<Trail items={[search(3)]} settled />);
		expect(few.match(/class="source-card"/g)?.length).toBe(3);
		expect(few).not.toContain("source-more");
		const many = renderToStaticMarkup(<Trail items={[search(29)]} settled />);
		expect(many).toContain("Sources · 29");
		expect(many.match(/class="source-card"/g)?.length).toBe(4);
		expect(many).toContain("Show 25 more");
	});

	test("a title that arrived as markdown reads as its words, and one that is only a logo has none", () => {
		expect(plainTitle("[![](/runta-logo.png)](/) Introducing [FrontierHarness](/eval) Eval")).toBe("Introducing FrontierHarness Eval");
		expect(plainTitle("[![](/runta-logo.png)](/frontier-harness-eval)")).toBeUndefined();
		expect(plainTitle("[![](/runta-logo.png)](/frontier-harness-")).toBeUndefined();
		expect(plainTitle("Codex vs Claude Code")).toBe("Codex vs Claude Code");
	});
});

describe("a command opened in the pane", () => {
	const ran = (text: string) => ({ kind: "tool", id: "r1", ts: 1, toolCallId: "r1", title: "bash: sed -n '170,210p' page.txt", status: "completed", output: [{ type: "text", text }] }) satisfies ToolStep;

	test("a harness's JSON result gives what it was for, its exit code and what it printed", () => {
		const result = ranOf(ran(JSON.stringify({ command: "sed -n '170,210p' page.txt", description: "Read Composio results table", exit_code: 0, output: "## Overall results\n| Harness |" })));
		expect(result).toEqual({ why: "Read Composio results table", exit: 0, printed: "## Overall results\n| Harness |" });
	});

	test("plain output is not taken for one", () => {
		expect(ranOf(ran("172:| Pi Agent | 156.2s |"))).toBeNull();
		expect(ranOf(ran('{"rows": 3}'))).toBeNull();
	});
});

test("a result the desk kept cut short still gives what it was for and what it printed so far", () => {
	const cut = '{\n  "command": "grep -n x page.txt",\n  "description": "Find median times in Composio page",\n  "exit_code": 0,\n  "output": "172:| Pi Agent | 156.2s |\\n174:| Hermes Agent | 16';
	const step = { kind: "tool", id: "r2", ts: 1, toolCallId: "r2", title: "bash: grep -n x page.txt", status: "completed", output: [{ type: "text", text: cut }] } satisfies ToolStep;
	expect(ranOf(step)).toEqual({ why: "Find median times in Composio page", exit: 0, printed: "172:| Pi Agent | 156.2s |\n174:| Hermes Agent | 16…" });
});
