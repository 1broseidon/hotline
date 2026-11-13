import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { TranscriptEvent, WebSource } from "../src/generated/contract";
import type { Handlers } from "../src/wire";
import { installDom } from "./dom";

// The sources of a reply on the line under it (epic-42, George 2026-11-12):
// outcomes, then the sites read as a stack of their icons, then the time; and
// the path lists them by name, each opening in the person's browser.

let act: typeof import("react").act;
let Transcript: typeof import("../src/components/Transcript").Transcript;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let restore: (() => void)[] = [];
let desks = 0;
/** The icon the desk keeps for each site; a site not here has none. */
let icons: Record<string, string>;

const icon = (site: string) => `data:image/png;base64,${btoa(site)}`;

beforeEach(async () => {
	const installed = installDom({ requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	Object.assign(globalThis, { ResizeObserver: installed.dom.ResizeObserver });
	act = (await import("react")).act;
	({ Transcript } = await import("../src/components/Transcript"));
	const { Wire, wire } = await import("../src/wire");
	const { replaceDesks } = await import("../src/desks");
	const { forgetPreviews } = await import("../src/linkPreview");
	forgetPreviews();
	icons = {};
	const subscribe = spyOn(Wire.prototype, "subscribe").mockImplementation(((target: unknown, handlers: Handlers<unknown, unknown>) => {
		if (target === "room") handlers.snapshot([]);
		return () => {};
	}) as InstanceType<typeof Wire>["subscribe"]);
	// The desk hands over each site's icon it keeps; anything else is still on its way.
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		if (cmd === "link.icon") return icons[(params as { site: string }).site] ?? null;
		return new Promise(() => {});
	}) as unknown as typeof wire.command);
	restore = [() => subscribe.mockRestore(), () => command.mockRestore()];
	replaceDesks([{ id: `line-sources-${++desks}`, kind: "local", name: "Test", origin: "ws://localhost", token: "test" }]);
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	for (const undo of restore) undo();
	await teardown();
});

async function draw(events: TranscriptEvent[], extra: Partial<Parameters<typeof Transcript>[0]> = {}) {
	await act(async () => root.render(<Transcript personaId="quill" name="Quill" events={events} streaming={[]} live={false} focus={null} {...extra} />));
	// The icons the line asked for arrive.
	await act(async () => new Promise((done) => setTimeout(done, 0)));
}

/** A page read: a fetch that recorded it. */
const read = (id: string, ts: number, url: string, title?: string): TranscriptEvent => ({
	kind: "tool", id, ts, endedTs: ts + 1_000, toolCallId: id, title: `Fetch ${url}`, toolKind: "fetch", status: "completed",
	sources: [{ url, ...(title !== undefined ? { title } : {}) }],
});
/** A search: its query, and the pages it turned up. */
const search = (id: string, ts: number, query: string, found: WebSource[] = []): TranscriptEvent => ({
	kind: "tool", id, ts, endedTs: ts + 1_000, toolCallId: id, title: `web_search: ${query}`, toolKind: "search", status: "completed",
	...(found.length > 0 ? { sources: found.map((page) => ({ ...page, found: true })) } : {}),
});
/** A local file read, and a command. */
const local = (id: string, ts: number, title: string, toolKind: string): TranscriptEvent => ({
	kind: "tool", id, ts, endedTs: ts + 1_000, toolCallId: id, title, toolKind, status: "completed",
});
const outcome = (id: string, ts: number, message: string): TranscriptEvent => ({ kind: "outcome", id, ts, type: "made", message, key: id });

const turn = (work: TranscriptEvent[]): TranscriptEvent[] => [
	{ kind: "user", id: "u", ts: 0, text: "What happened with the storm last night?" },
	...work,
	{ kind: "agent", id: "a", ts: 40_000, text: "Two tornado warnings, no injuries reported." },
	{ kind: "turn", id: "end", ts: 40_001, stopReason: "end_turn" },
];

const line = () => container.querySelector<HTMLButtonElement>(".steps-caption")!;
const stack = () => line().querySelector<HTMLElement>(".favicons");
/** The line's parts in order, by what each is. */
const parts = () => [...line().children].map((part) => (part.classList.contains("favicons") ? "sources" : part.classList.contains("outcome") ? "outcome" : part.className));

describe("the line under a reply", () => {
	test("sources and no outcomes: the stack of the sites read, then the time", async () => {
		for (const site of ["cbsnews.com", "spectrumnews.com", "apnews.com", "weather.gov"]) icons[site] = icon(site);
		await draw(
			turn([
				read("r1", 1_000, "https://www.cbsnews.com/storm-live", "Live: storms across the region"),
				read("r2", 5_000, "https://spectrumnews.com/weather/tornado"),
				read("r3", 9_000, "https://apnews.com/article/storms"),
				read("r4", 13_000, "https://www.weather.gov/warnings"),
			]),
		);
		expect(parts()).toEqual(["sources", "steps-caption-took"]);
		expect([...stack()!.querySelectorAll("img")].map((img) => img.getAttribute("src"))).toEqual([icon("cbsnews.com"), icon("spectrumnews.com"), icon("apnews.com")]);
		expect(stack()!.querySelector(".favicons-more")?.textContent).toBe("+1");
		expect(line().querySelector(".steps-caption-took")?.textContent).toBe("13s");
		// No step count, and the sources carry no outcome's mark.
		expect(line().textContent).not.toMatch(/step/);
		expect(stack()!.closest(".outcome")).toBeNull();
	});

	test("with both, the outcomes come first, then the sources, then the time", async () => {
		icons["cbsnews.com"] = icon("cbsnews.com");
		await draw(turn([read("r1", 1_000, "https://cbsnews.com/storm"), outcome("o1", 2_000, "Storm summary for the family chat")]));
		expect(parts()).toEqual(["outcome", "sources", "steps-caption-took"]);
	});

	test("with neither, the time alone", async () => {
		await draw(turn([local("f1", 1_000, "Read /Users/ana/notes/storm.md", "read"), local("c1", 3_000, "Run ls ~/Downloads", "execute")]));
		expect(parts()).toEqual(["steps-caption-took"]);
		expect(line().textContent).toBe("3.0s");
	});

	test("each site once, in the order first read, pages read before what searches only found, and +N counts the other sites", async () => {
		for (const site of ["apnews.com", "cbsnews.com", "nws.noaa.gov", "kxan.com", "fox7austin.com"]) icons[site] = icon(site);
		await draw(
			turn([
				search("s1", 500, "austin storm last night", [{ url: "https://kxan.com/weather" }, { url: "https://apnews.com/article/storms" }, { url: "https://fox7austin.com/news" }]),
				read("r1", 1_000, "https://apnews.com/article/storms"),
				read("r2", 3_000, "https://www.cbsnews.com/a"),
				read("r3", 5_000, "https://apnews.com/article/other"),
				read("r4", 7_000, "https://cbsnews.com/b"),
				read("r5", 9_000, "https://nws.noaa.gov/x"),
			]),
		);
		expect([...stack()!.querySelectorAll("img")].map((img) => img.getAttribute("src"))).toEqual([icon("apnews.com"), icon("cbsnews.com"), icon("nws.noaa.gov")]);
		// Five sites, three drawn: two more, not the seven pages less three.
		expect(stack()!.querySelector(".favicons-more")?.textContent).toBe("+2");
		expect(stack()!.getAttribute("aria-label")).toBe("Sources: apnews.com, cbsnews.com, nws.noaa.gov and 2 more");
	});

	test("the stack is named in words: a short tooltip, and the whole for a screen reader", async () => {
		for (const site of ["cbsnews.com", "spectrumnews.com", "apnews.com"]) icons[site] = icon(site);
		const sites = ["cbsnews.com", "spectrumnews.com", "apnews.com", "weather.gov", "kxan.com", "bbc.com"];
		await draw(turn(sites.map((site, at) => read(`r${at}`, 1_000 + at * 1_000, `https://${site}/storm`))));
		expect(stack()!.getAttribute("role")).toBe("img");
		expect(stack()!.getAttribute("aria-label")).toBe("Sources: cbsnews.com, spectrumnews.com, apnews.com and 3 more");
		expect(stack()!.getAttribute("title")).toBe("cbsnews.com and 5 more");
		const { sitesSaid } = await import("../src/turnFacts");
		expect(sitesSaid(["cbsnews.com"])).toEqual({ label: "Sources: cbsnews.com", tip: "cbsnews.com" });
		expect(sitesSaid(["cbsnews.com", "apnews.com"])).toEqual({ label: "Sources: cbsnews.com and apnews.com", tip: "cbsnews.com and apnews.com" });
		expect(sitesSaid(["cbsnews.com", "apnews.com", "bbc.com"])).toEqual({ label: "Sources: cbsnews.com, apnews.com and bbc.com", tip: "cbsnews.com and 2 more" });
	});

	test("a site with no icon draws none and is counted in +N; with none at all, the first site is named in words", async () => {
		icons["apnews.com"] = icon("apnews.com");
		await draw(turn([read("r1", 1_000, "https://cbsnews.com/a"), read("r2", 2_000, "https://apnews.com/b"), read("r3", 3_000, "https://bbc.com/c")]));
		expect(stack()!.querySelectorAll("img")).toHaveLength(1);
		expect(stack()!.querySelector(".favicons-more")?.textContent).toBe("+2");
		const { forgetPreviews } = await import("../src/linkPreview");
		forgetPreviews();
		icons = {};
		await draw(turn([read("r1", 1_000, "https://cbsnews.com/a"), read("r2", 2_000, "https://apnews.com/b")]));
		expect(stack()!.querySelectorAll("img")).toHaveLength(0);
		expect(stack()!.textContent).toBe("cbsnews.com+1");
		expect(stack()!.getAttribute("aria-label")).toBe("Sources: cbsnews.com and apnews.com");
	});

	test("a search's query is not a source, nor a local file or a page on this machine's disk; a page read is", async () => {
		icons["cbsnews.com"] = icon("cbsnews.com");
		await draw(
			turn([
				search("s1", 500, "storm last night"),
				local("f1", 1_000, "Read /Users/ana/notes/storm.md", "read"),
				read("f2", 2_000, "file:///Users/ana/notes/storm.html"),
				read("r1", 3_000, "https://cbsnews.com/storm"),
			]),
		);
		expect(stack()!.getAttribute("aria-label")).toBe("Sources: cbsnews.com");
		expect(stack()!.querySelector(".favicons-more")).toBeNull();
		// With none of those and no page read, there is no stack.
		await draw(turn([search("s1", 500, "storm last night"), local("f1", 1_000, "Read /Users/ana/notes/storm.md", "read")]));
		expect(stack()).toBeNull();
	});

	test("while the turn is still on it there is no line, so no stack", async () => {
		icons["cbsnews.com"] = icon("cbsnews.com");
		await draw([{ kind: "user", id: "u", ts: 0, text: "Storm?" }, read("r1", 1_000, "https://cbsnews.com/storm")], { live: true });
		expect(container.querySelector(".steps-caption")).toBeNull();
		expect(container.querySelector(".favicons")).toBeNull();
	});
});

describe("the path lists the sources by name, each opening in the browser", () => {
	test("opened in place, as in a thread, the path heads its steps with the pages read: site, title, link", async () => {
		const native = await import("../src/native");
		const opened: string[] = [];
		const open = spyOn(native, "openLink").mockImplementation(async (url: string) => {
			opened.push(url);
		});
		restore.push(() => open.mockRestore());
		await draw(turn([read("r1", 1_000, "https://www.cbsnews.com/storm-live", "Live: storms across the region"), read("r2", 5_000, "https://apnews.com/article/storms")]));
		await act(async () => line().click());
		const sources = container.querySelector('.steps > section[aria-label="Sources"]')!;
		expect(sources.querySelector(".work-heading")?.textContent).toBe("Sources · 2");
		const cards = [...sources.querySelectorAll<HTMLAnchorElement>("a.source-card")];
		expect(cards.map((card) => [card.querySelector(".source-site")?.textContent, card.querySelector(".source-title")?.textContent, card.getAttribute("href")])).toEqual([
			["cbsnews.com", "Live: storms across the region", "https://www.cbsnews.com/storm-live"],
			["apnews.com", "/article/storms", "https://apnews.com/article/storms"],
		]);
		await act(async () => cards[1]!.click());
		expect(opened).toEqual(["https://apnews.com/article/storms"]);
		// The sources head the path: the steps come after them.
		expect(sources.nextElementSibling).not.toBeNull();
	});

	test("a path with no pages read has no Sources group", async () => {
		await draw(turn([local("c1", 1_000, "Run ls", "execute")]));
		await act(async () => line().click());
		expect(container.querySelector('.steps [aria-label="Sources"]')).toBeNull();
	});

	test("in the pane, the path's top lists them the same way, and a card opens its page", async () => {
		const native = await import("../src/native");
		const opened: string[] = [];
		const open = spyOn(native, "openLink").mockImplementation(async (url: string) => {
			opened.push(url);
		});
		restore.push(() => open.mockRestore());
		const { Trail } = await import("../src/components/transcript/Trail");
		const page = read("r1", 1_000, "https://cbsnews.com/storm", "Storm live");
		if (page.kind !== "tool") throw new Error("a page read is a step");
		await act(async () => root.render(<Trail items={[page]} settled />));
		const card = container.querySelector<HTMLAnchorElement>('section[aria-label="Sources"] a.source-card')!;
		expect(card.querySelector(".source-title")?.textContent).toBe("Storm live");
		await act(async () => card.click());
		expect(opened).toEqual(["https://cbsnews.com/storm"]);
	});
});
