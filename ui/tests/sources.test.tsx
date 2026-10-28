import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { Step } from "../src/scheduledRuns";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Trail, plainTitle } = await import("../src/components/transcript/Trail");

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
