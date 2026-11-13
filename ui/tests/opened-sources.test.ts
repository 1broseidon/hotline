import { describe, expect, test } from "bun:test";
import type { WebSource } from "../src/generated/contract";
import { localAddress, sourcesOf, type ToolStep } from "../src/turnFacts";

// A turn's sources are the pages it opened, never what a search only listed,
// and never a page on this machine (George, 2026-11-12). Each step here is a
// shape found on George's dev desk, its tape and line named beside it.

let at = 0;
const step = (title: string, toolKind: string, output: string, extra: Partial<ToolStep> = {}): ToolStep => ({
	kind: "tool",
	id: `s${++at}`,
	ts: at,
	toolCallId: `c${at}`,
	title,
	toolKind,
	status: "completed",
	...(output !== "" ? { output: [{ type: "text", text: output }] } : {}),
	...extra,
});
const listed = (title: string, pages: WebSource[]) => step(title, "web_search", "", { sources: pages.map((page) => ({ ...page, found: true })) });
const opened = (items: ToolStep[]) => sourcesOf(items).map((source) => [source.url, source.title]);

describe("what a turn opened is a source", () => {
	test("its computer's browser, before the desk recorded it: the address it navigated to or opened a tab at (Goldie, 79d12f56 l.3048)", () => {
		expect(
			opened([
				step("computer__browser", "computer__browser", "navigated to https://nvidianews.nvidia.com/news/nvidia-announces-ceo-transition"),
				step("computer__browser", "computer__browser", "page: NVIDIA Announces CEO Transition | NVIDIA Newsroom\nNewsroom"),
				step("computer__browser", "computer__browser", "opened new tab at https://porkbun.com/account/login; the server answered with an error status; browser text shows the page"),
			]),
		).toEqual([
			["https://nvidianews.nvidia.com/news/nvidia-announces-ceo-transition", undefined],
			["https://porkbun.com/account/login", undefined],
		]);
	});

	test("the same browser through an ACP agent's MCP client, by its tool's name (Claude Code, 79d12f56 l.2907)", () => {
		expect(opened([step("mcp__Computer__browser", "other", "navigated to https://github.com/notifications")])).toEqual([["https://github.com/notifications", undefined]]);
	});

	test("the desk's browser, its page and title at the head of its answer, an act on the same page once (1e615299 l.176, 9f959998 l.225)", () => {
		expect(
			opened([
				step("mcp__hotline__browser", "other", "Page: Apple I - Wikipedia\nURL: https://en.wikipedia.org/wiki/Apple_I"),
				step("mcp__hotline__browser", "other", "Scrolled.\nPage: Apple I - Wikipedia\nURL: https://en.wikipedia.org/wiki/Apple_I\nRead the page again with text to see what changed."),
				step("browser", "browser", "Page: Juno - NASA Science\nURL: https://science.nasa.gov/mission/juno/"),
				step("browser", "browser", "Page: (no title)\nURL: https://example.org/blank"),
			]),
		).toEqual([
			["https://en.wikipedia.org/wiki/Apple_I", "Apple I - Wikipedia"],
			["https://science.nasa.gov/mission/juno/", "Juno - NASA Science"],
			["https://example.org/blank", undefined],
		]);
	});

	test("what the desk recorded is taken as it is, and an act that named no address opened nothing", () => {
		expect(
			opened([
				step("computer__browser", "computer__browser", "navigated to https://www.cbsnews.com/news/nvidia-ceo-jensen-huang-resigns/", { sources: [{ url: "https://www.cbsnews.com/news/nvidia-ceo-jensen-huang-resigns/" }] }),
				step("computer__browser", "computer__browser", "clicked e12"),
				step("computer__browser", "computer__browser", "back"),
				step("computer__browser", "computer__browser", '[{"current":true,"index":0,"title":"porkbun.com","url":"https://porkbun.com/"}]'),
			]),
		).toEqual([["https://www.cbsnews.com/news/nvidia-ceo-jensen-huang-resigns/", undefined]]);
	});

	test("an ACP agent's own fetch: Claude Code's and Cursor's, recorded, and Claude's from before the desk named pages (1e615299 l.27, 435e51a4 l.300)", () => {
		expect(
			opened([
				step("Fetch https://lapu.ai/blog/desktop-computer-use-agents-compared", "fetch", "# Desktop AI Agents: Summary by Tool", { sources: [{ url: "https://lapu.ai/blog/desktop-computer-use-agents-compared" }] }),
				step("Web Fetch: https://rentamac.io/mac-mini-history/", "fetch", "", { sources: [{ url: "https://rentamac.io/mac-mini-history/" }] }),
				step("Fetch https://upstash.com/blog/cloud-computer-for-ai-agents", "fetch", "The source covers isolation, persistence, pricing"),
			]),
		).toEqual([
			["https://lapu.ai/blog/desktop-computer-use-agents-compared", undefined],
			["https://rentamac.io/mac-mini-history/", undefined],
			["https://upstash.com/blog/cloud-computer-for-ai-agents", undefined],
		]);
	});

	test("a step that failed opened nothing, and a harness's own words for a call are not its answer (Antigravity, 9f959998 l.207)", () => {
		expect(
			opened([
				step("mcp__Computer__browser", "other", "navigated to https://example.com/", { status: "failed" }),
				step("hotline_browser", "other", "Navigate to github.com"),
				step("hotline_browser", "other", "Navigate to http://127.0.0.1:4520/"),
			]),
		).toEqual([]);
	});
});

describe("what a search only listed is not", () => {
	const results = [
		{ url: "https://www.cbsnews.com/news/nvidia-ceo-jensen-huang-resigns/", title: "Nvidia CEO Jensen Huang resigns, board announces - CBS News" },
		{ url: "https://www.bbc.com/news/live/c6zjry11v8w1t", title: "Huang agrees to appear before Congress" },
	];

	test("a listing never opened is no source", () => {
		expect(sourcesOf([listed("web_search Jensen Huang latest news", results)])).toEqual([]);
	});

	test("a listing opened later is, with the listing's title when the opening gave none", () => {
		expect(opened([listed("web_search Jensen Huang latest news", results), step("computer__browser", "computer__browser", `navigated to ${results[0]!.url}`)])).toEqual([[results[0]!.url, results[0]!.title]]);
	});

	test("a page opened before a search listed it keeps its place, and its own title over the listing's", () => {
		expect(
			opened([
				step("web_fetch: https://www.bbc.com/news/live/c6zjry11v8w1t", "fetch", "", { sources: [{ url: results[1]!.url, title: "Huang resigns: live" }] }),
				step("computer__browser", "computer__browser", `navigated to ${results[0]!.url}`),
				listed("web_search Jensen Huang", results),
			]),
		).toEqual([
			[results[1]!.url, "Huang resigns: live"],
			[results[0]!.url, results[0]!.title],
		]);
	});
});

describe("a page on this machine is not a source", () => {
	test("the phone's rule: 127.x, localhost and *.localhost, ::1, 0.0.0.0, *.local", () => {
		for (const url of ["http://127.0.0.1:8080/phone.html", "http://127.1.2.3/", "http://localhost:4520/", "https://app.localhost/", "http://[::1]:3000/", "http://[0:0:0:0:0:0:0:1]/", "http://0.0.0.0:8000/", "http://printer.local/"]) {
			expect(localAddress(url)).toBe(true);
		}
		for (const url of ["https://localhost.example.com/", "https://www.bbc.com/news", "http://10.0.0.5/", "https://local.news/"]) {
			expect(localAddress(url)).toBe(false);
		}
	});

	test("opened in a browser or fetched, it is left out, as is a file (79d12f56 l.2764, l.2468)", () => {
		expect(
			opened([
				step("mcp__Computer__browser", "other", "navigated to http://127.0.0.1:8080/phone.html"),
				step("mcp__Computer__browser", "other", "navigated to file:///home/agent/gull-point/index.html"),
				step("mcp__hotline__browser", "other", "Page: Hotline: an open-source personal agent ecosystem\nURL: http://127.0.0.1:4520/"),
				step("Fetch http://localhost:4173/upload", "fetch", "", { sources: [{ url: "http://localhost:4173/upload" }] }),
				step("curl -s http://[::1]:8787/health", "execute", "", { sources: [{ url: "http://[::1]:8787/health" }] }),
				step("Fetch https://hotline.dev/", "fetch", "", { sources: [{ url: "https://hotline.dev/" }] }),
			]),
		).toEqual([["https://hotline.dev/", undefined]]);
	});
});
