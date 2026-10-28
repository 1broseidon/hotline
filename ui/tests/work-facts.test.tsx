import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Activity } from "../src/activity";
import type { Step } from "../src/scheduledRuns";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const real = await import("../src/voice/transcription");
mock.module("../src/voice/transcription", () => ({ ...real, hearsOnThisMac: async () => true }));

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

beforeEach(async () => {
	({ teardown } = installDom());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

type Tool = Extract<Step, { kind: "tool" }>;
let at = 0;
const step = (title: string, toolKind: string, said: string, extra: Partial<Tool> = {}): Tool => ({
	kind: "tool",
	id: `s${++at}`,
	ts: at * 1000,
	toolCallId: `c${at}`,
	title,
	toolKind,
	status: "completed",
	said: { key: `${toolKind}:${said}`, one: said, many: `${said} ({n})`, doing: said },
	...extra,
});
const fetched = (url: string, title: string, took: number) =>
	step(`Fetch ${url}`, "fetch", "Read a page", { said: { key: "fetch", one: "Read a page", many: "Read {n} web pages", doing: "Reading" }, endedTs: at * 1000 + 1000 + took, sources: [{ url, title }] });

/** A turn as a 0.45 desk records it: a search, two pages read, two edits counted, a test run, and a failure. */
function recorded(): Step[] {
	at = 0;
	return [
		step('Search "release blockers"', "fetch", "Searched the web", {
			endedTs: 1400,
			sources: [
				{ url: "https://nextjs.org/blog/next-15-4", title: "Next.js 15.4", found: true },
				{ url: "https://www.github.com/arc-ui/web/issues", title: "Issues", found: true },
				{ url: "https://vercel.com/changelog", found: true },
			],
		}),
		fetched("https://nextjs.org/blog/next-15-4", "Next.js 15.4: Turbopack builds are stable", 1200),
		fetched("https://stackoverflow.com/questions/1/safari-18", "Safari 18 file input upload fails silently", 1800),
		step("Edit src/upload/Uploader.tsx", "edit", "Edited Uploader.tsx", { said: { key: "edit", one: "Edited a file", many: "Edited {n} files", doing: "Editing" }, files: [{ path: "src/upload/Uploader.tsx", created: false, added: 18, removed: 4 }] }),
		step("Edit src/upload/safari.ts", "edit", "Edited safari.ts", { said: { key: "edit", one: "Edited a file", many: "Edited {n} files", doing: "Editing" }, files: [{ path: "src/upload/safari.ts", created: true, added: 31, removed: 0 }] }),
		step("cargo test -p api", "execute", "Ran cargo test", { output: [{ type: "text", text: "test result: ok. 212 passed; 0 failed" }], endedTs: 30_000 }),
		step("psql -c 'select 1'", "execute", "Query pending migrations", { status: "failed", output: [{ type: "text", text: "psql: error: timeout expired" }], endedTs: 40_000 }),
	];
}

/** The same turn as George's 0.44 desk writes it: no ends, no sources, no line counts. */
function before045(): Step[] {
	return recorded().map((one) => {
		if (one.kind !== "tool") return one;
		const { endedTs: _end, sources: _sources, ...rest } = one;
		return rest.files === undefined ? rest : { ...rest, files: rest.files.map(({ path, created }) => ({ path, created })) };
	});
}

describe("a turn's work in words and counts", () => {
	test("the done summary counts steps, distinct sources, files and failures", async () => {
		const { summaryOf, summaryWords } = await import("../src/turnFacts");
		const summary = summaryOf(recorded());
		expect(summary).toEqual({ steps: 7, sources: 4, files: 2, failed: 1 });
		expect(summaryWords(summary)).toEqual({ said: "7 steps · 4 sources · 2 files", failed: "1 failed" });
		// The history line has the sites' icons for its sources.
		expect(summaryWords(summary, { sources: false }).said).toBe("7 steps · 2 files");
	});

	test("each line's fact is one the steps recorded: results, lines in and out, tests passed, a time, a timeout", async () => {
		const { glance } = await import("../src/glance");
		const { factOf } = await import("../src/turnFacts");
		const lines = glance(recorded(), false);
		expect(lines.map((line) => [line.text, factOf(line.steps, line.live)?.text])).toEqual([
			["Searched the web", "3 results"],
			["Read 2 web pages", "3.0s"],
			["Edited 2 files", "+49 −4"],
			["Ran cargo test", "212 passed"],
			["Query pending migrations", "timeout"],
		]);
		expect(factOf(lines[4]!.steps, false)?.tone).toBe("danger");
		expect(factOf(lines[0]!.steps, true)).toEqual({ text: "now", tone: "live" });
	});

	test("a turn from a 0.44 desk keeps its words and drops the facts it never recorded", async () => {
		const { glance } = await import("../src/glance");
		const { factOf, filesOf, sourcesOf, summaryOf } = await import("../src/turnFacts");
		const old = before045();
		// The pages its fetches name in their titles are still its sources, without titles; its search results are not known.
		expect(sourcesOf(old).map((one) => [one.site, one.title])).toEqual([
			["nextjs.org", undefined],
			["stackoverflow.com", undefined],
		]);
		expect(filesOf(old)).toEqual([
			{ path: "src/upload/Uploader.tsx", created: false },
			{ path: "src/upload/safari.ts", created: true },
		]);
		expect(summaryOf(old)).toEqual({ steps: 7, sources: 2, files: 2, failed: 1 });
		const facts = glance(old, false).map((line) => factOf(line.steps, false)?.text ?? null);
		expect(facts).toEqual([null, null, null, "212 passed", "timeout"]);
	});
});

describe("the pane's sections, built from a recorded turn", () => {
	test("Sources as cards with the page's title, Files changed with their lines, then Steps", async () => {
		const { Trail } = await import("../src/components/transcript/Trail");
		await act(async () => root.render(<Trail items={recorded()} settled />));
		const headings = [...container.querySelectorAll(".work-heading")].map((one) => one.textContent);
		expect(headings).toEqual(["Sources · 4", "Files changed · 2", "Steps · 7"]);
		const cards = [...container.querySelectorAll<HTMLAnchorElement>("a.source-card")];
		expect(cards.map((card) => card.querySelector(".source-site")?.textContent)).toEqual(["nextjs.org", "stackoverflow.com", "github.com", "vercel.com"]);
		expect(cards[0]!.querySelector(".source-title")?.textContent).toBe("Next.js 15.4: Turbopack builds are stable");
		// A page with no title of its own says where on the site it is.
		expect(cards[3]!.querySelector(".source-title")?.textContent).toBe("/changelog");
		const files = [...container.querySelectorAll(".file-row")].map((row) => row.textContent);
		expect(files).toEqual(["src/upload/Uploader.tsx+18 −4", "src/upload/safari.ts+31 −0"]);
	});

	test("a failed line opens to its command and its error", async () => {
		const { Trail } = await import("../src/components/transcript/Trail");
		await act(async () => root.render(<Trail items={recorded()} settled />));
		const failed = container.querySelector<HTMLButtonElement>('.trail-step[data-status="failed"] .trail-head')!;
		expect(failed.querySelector(".trail-fact")?.textContent).toBe("timeout");
		await act(async () => failed.click());
		const labels = [...container.querySelectorAll('.trail-step[data-status="failed"] .trail-label')].map((one) => one.textContent);
		expect(labels).toEqual(["Command", "Error"]);
		expect(container.querySelector('.trail-step[data-status="failed"] .step-out')?.textContent).toContain("timeout expired");
	});

	test("from a 0.44 desk: no titles, no line counts, no empty sections", async () => {
		const { Trail } = await import("../src/components/transcript/Trail");
		at = 0;
		const old = before045().filter((one) => one.kind === "tool" && one.toolKind === "execute");
		await act(async () => root.render(<Trail items={old} settled />));
		expect([...container.querySelectorAll(".work-heading")].map((one) => one.textContent)).toEqual(["Steps · 2"]);
	});
});

describe("the composer's row, in its five states", () => {
	async function row(work: { activity: Activity; count?: number; done?: { steps: number; sources: number; files: number; failed: number } }) {
		const { Composer } = await import("../src/components/Composer");
		await act(async () =>
			root.render(
				<Composer
					personaId="ada"
					name="Ada"
					state="thinking"
					replyQuote={null}
					onSend={() => {}}
					onCancel={() => {}}
					onClearReply={() => {}}
					work={{ sleeping: false, open: false, onToggle: () => {}, ...work }}
				/>,
			),
		);
		return container.querySelector<HTMLButtonElement>("button.composer-work")!;
	}

	test("Thinking: the mark and the word", async () => {
		const button = await row({ activity: { phase: "thinking", word: "Thinking" } });
		expect(button.querySelector(".composer-work-word")?.textContent).toBe("Thinking");
		expect(button.querySelector(".composer-work-count")).toBeNull();
	});

	test("Working: what it is doing now and how many steps so far", async () => {
		const button = await row({ activity: { phase: "doing", word: "Searching issues in arc-ui/web" }, count: 5 });
		expect(button.querySelector(".composer-work-word")?.textContent).toBe("Searching issues in arc-ui/web");
		expect(button.querySelector(".composer-work-count")?.textContent).toBe("5");
	});

	test("Reading the web: the site, and the icons of the sites read so far", async () => {
		const button = await row({ activity: { phase: "search", word: "Reading vercel.com", sites: ["nextjs.org", "github.com", "vercel.com"] }, count: 4 });
		expect(button.querySelector(".composer-work-word")?.textContent).toBe("Reading vercel.com");
		expect([...button.querySelectorAll("img.favicon")].map((icon) => icon.getAttribute("src"))).toEqual([
			"https://nextjs.org/favicon.ico",
			"https://github.com/favicon.ico",
			"https://vercel.com/favicon.ico",
		]);
	});

	test("Needs you: amber, with what waits", async () => {
		const button = await row({ activity: { phase: "blocked", word: "Needs you · Allow rm -rf build" } });
		expect(button.hasAttribute("data-blocked")).toBe(true);
		expect(button.querySelector(".composer-work-word")?.textContent).toBe("Needs you · Allow rm -rf build");
	});

	test("Done: the counts, the failure apart, and no mark", async () => {
		const button = await row({ activity: { phase: "rest", word: "" }, done: { steps: 12, sources: 4, files: 3, failed: 1 } });
		expect(button.querySelector(".composer-work-word")?.textContent).toBe("12 steps · 4 sources · 3 files");
		expect(button.querySelector(".composer-work-failed")?.textContent).toBe("1 failed");
		expect(button.querySelector(".glyph")).toBeNull();
	});
});
