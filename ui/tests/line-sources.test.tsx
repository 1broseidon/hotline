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

	test("each site once, in the order first opened; a search's listing is not a source, and +N counts the other sites opened", async () => {
		for (const site of ["apnews.com", "cbsnews.com", "nws.noaa.gov", "weather.gov", "kxan.com", "fox7austin.com"]) icons[site] = icon(site);
		await draw(
			turn([
				search("s1", 500, "austin storm last night", [{ url: "https://kxan.com/weather" }, { url: "https://apnews.com/article/storms" }, { url: "https://fox7austin.com/news" }]),
				read("r1", 1_000, "https://apnews.com/article/storms"),
				read("r2", 3_000, "https://www.cbsnews.com/a"),
				read("r3", 5_000, "https://apnews.com/article/other"),
				read("r4", 7_000, "https://cbsnews.com/b"),
				read("r5", 9_000, "https://nws.noaa.gov/x"),
				read("r6", 11_000, "https://weather.gov/y"),
			]),
		);
		expect([...stack()!.querySelectorAll("img")].map((img) => img.getAttribute("src"))).toEqual([icon("apnews.com"), icon("cbsnews.com"), icon("nws.noaa.gov")]);
		// Four sites opened, three drawn: one more, not the six pages less three, and nothing for kxan or fox7austin, only listed.
		expect(stack()!.querySelector(".favicons-more")?.textContent).toBe("+1");
		expect(stack()!.getAttribute("aria-label")).toBe("Sources: apnews.com, cbsnews.com, nws.noaa.gov and 1 more");
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

/**
 * Goldie's turn on George's dev desk, 2026-11-12 20:17 (tape 79d12f56…, lines
 * 3036-3059), as it was written, its long outputs cut: Hotline Agent on a GPT
 * model searched once (six results), read its computer skill, checked its
 * computer twice, and in its computer's browser opened two of the six and
 * read each. Written before the desk recorded a browser's page, so its
 * browser steps carry no sources: what they opened is read off their answers.
 */
const GOLDIE: TranscriptEvent[] = [
	{"kind": "user", "id": "e39751ed-49a5-4361-9756-f54d774404fd", "ts": 1794536264357, "text": "whats your take on the latest news about jensen"},
	{"kind": "agent", "id": "b3803189-9700-41fa-a8a4-1e4dfe3c0565", "ts": 1794536271261, "text": "I’ll check the latest on Jensen Huang."},
	{"kind": "tool", "id": "tool:9d69f0e1-6b4c-44d5-a8b3-885787b85f9c", "ts": 1794536271262, "toolCallId": "9d69f0e1-6b4c-44d5-a8b3-885787b85f9c", "title": "web_search Jensen Huang latest news November 12 2026", "toolKind": "web_search", "status": "completed", "output": [{"type": "text", "text": "1. Nvidia CEO Jensen Huang resigns, board announces - CBS News\n   https://www.cbsnews.com/news/nvidia-ceo-jensen-huang-resigns/\n   Nvidia CEO Jensen Huang resigns in wake of affair with woman suspected of spying for China By Alex Sundby Updated on: November 12, 2026 / 5:28 PM EST / CBS News Add CBS News on Google\n\n2. Huang agrees to appear before Congress, US lawmakers say, after ...\n   https://www.bbc.com/news/live/c6zjry11v8w1t\n   12 November 2026 · Jensen Huang has resigned as CEO of tech giant Nvidia, the company's board says, with chief financial officer Colette Kress appointed interim boss · US lawmakers on Capitol Hill say Huang has indicated he is willing to appear ...\n\n3. Nvidia faces its future without Jensen Huang, the AI pioneer\n   https://spectrumlocalnews.com/us/snplus/business/2026/11/12/jensen-huang-nvidia-ceo-future-uncertain\n   Jensen Huang built the company that powers AI. Now Nvidia must find a way forward without him BY Associated Press Nationwide UPDATED 4:34 PM ET Nov. 12, 2026\n\n4. NVIDIA Announces CEO Transition\n   https://nvidianews.nvidia.com/news/nvidia-announces-ceo-transition\n   12 hours ago ... NVIDIA Announces CEO Transition. November 12, 2026. Jensen Huang Resigns; Colette Kress, Executive Vice President and Chief Financial Officer, ...\n\n5. Stock Market News, Nov. 12, 2026: Chip Giant's Stock Falls 4% After ...\n   https://www.wsj.com/livecoverage/stock-market-today-dow-sp-500-nasdaq-11-12-2026\n   Nvidia CEO Jensen Huang resigned today, capping an era during which he stoked the global AI boom and became a geopolitical star. Shares in the chip giant tumbled as much as 14% before regaining ...\n\n6. What does Jensen Huang's resignation mean for AI? - USA TODAY\n   https://www.usatoday.com/story/tech/2026/11/12/what-does-jensen-huangs-resignation-mean-for-ai/92739989007/\n   Jensen Huang spent 33 years at the helm of Nvidia, co-founding the AI infrastructure company in 1993 and shaping it into one of the biggest companies in the world. On Nov. 12, his reign came to an ...\n\nSearched with SearXNG."}], "said": {"key": "web", "one": "Searched the web", "many": "Searched the web {n} times", "doing": "Searching the web"}, "endedTs": 1794536272105, "sources": [{"url": "https://www.cbsnews.com/news/nvidia-ceo-jensen-huang-resigns/", "title": "Nvidia CEO Jensen Huang resigns, board announces - CBS News", "found": true}, {"url": "https://www.bbc.com/news/live/c6zjry11v8w1t", "title": "Huang agrees to appear before Congress, US lawmakers say, after ...", "found": true}, {"url": "https://spectrumlocalnews.com/us/snplus/business/2026/11/12/jensen-huang-nvidia-ceo-future-uncertain", "title": "Nvidia faces its future without Jensen Huang, the AI pioneer", "found": true}, {"url": "https://nvidianews.nvidia.com/news/nvidia-announces-ceo-transition", "title": "NVIDIA Announces CEO Transition", "found": true}, {"url": "https://www.wsj.com/livecoverage/stock-market-today-dow-sp-500-nasdaq-11-12-2026", "title": "Stock Market News, Nov. 12, 2026: Chip Giant's Stock Falls 4% After ...", "found": true}, {"url": "https://www.usatoday.com/story/tech/2026/11/12/what-does-jensen-huangs-resignation-mean-for-ai/92739989007/", "title": "What does Jensen Huang's resignation mean for AI? - USA TODAY", "found": true}]},
	{"kind": "tool", "id": "tool:6af8e0b3-99ee-4796-a9f3-142122d15b82", "ts": 1794536284897, "toolCallId": "6af8e0b3-99ee-4796-a9f3-142122d15b82", "title": "read .agents/skills/hotline-computer/SKILL.md", "toolKind": "read", "status": "completed", "output": [{"type": "text", "text": "1|---\n2|name: hotline-computer\n3|description: Operate a Hotline Computer through its MCP tools for browser work, installing and testing GitHub tools, and building or testing native desktop apps in its container.\n4|---\n5|\n6|Read `state {\"act…"}], "said": {"key": "read", "one": "Read SKILL.md", "many": "Read {n} files", "doing": "Reading"}, "endedTs": 1794536284911},
	{"kind": "tool", "id": "tool:a0cec0c8-d832-42d4-af2d-196705b2f209", "ts": 1794536287896, "toolCallId": "a0cec0c8-d832-42d4-af2d-196705b2f209", "title": "computer__state", "toolKind": "computer__state", "status": "completed", "output": [{"type": "text", "text": "{\"architecture\":\"aarch64\",\"base\":\"0.10.0-rc.4\",\"build\":{\"channel\":\"development\",\"revision\":\"unknown\"},\"capabilities\":[\"capture\",\"input\",\"browser\",\"shell\",\"files\",\"windows\",\"wait\",\"state\"],\"catalog\":{\"nixpkgs\":\"ef34387ddd751e1ab8857adf467649…"}], "said": {"key": "computer", "one": "Checked its computer", "many": "Used the computer {n} times", "doing": "Checking its computer"}, "endedTs": 1794536287903},
	{"kind": "tool", "id": "tool:97e7a95d-74ab-494a-a2c3-0a13a27ef8e0", "ts": 1794536287903, "toolCallId": "97e7a95d-74ab-494a-a2c3-0a13a27ef8e0", "title": "computer__state", "toolKind": "computer__state", "status": "completed", "output": [{"type": "text", "text": "{\"build\":{\"channel\":\"development\",\"revision\":\"unknown\"},\"catalog\":{\"nixpkgs\":\"ef34387ddd751e1ab8857adf4676492d32eb24ec\",\"packages\":[{\"group\":\"Languages\",\"packages\":[{\"name\":\"python312\",\"note\":\"Python 3.12; uv or poetry for project environme…"}], "said": {"key": "computer", "one": "Checked its computer", "many": "Used the computer {n} times", "doing": "Checking its computer"}, "endedTs": 1794536287921},
	{"kind": "tool", "id": "tool:8a2d401b-275c-4751-bd73-8af7f5633f1a", "ts": 1794536291466, "toolCallId": "8a2d401b-275c-4751-bd73-8af7f5633f1a", "title": "computer__browser", "toolKind": "computer__browser", "status": "completed", "output": [{"type": "text", "text": "navigated to https://nvidianews.nvidia.com/news/nvidia-announces-ceo-transition"}], "said": {"key": "browser", "one": "Used the browser", "many": "Used the browser {n} times", "doing": "Browsing"}, "endedTs": 1794536292295},
	{"kind": "tool", "id": "tool:8f4d324f-a825-4f68-b5e9-f2dc5b7da6b6", "ts": 1794536295832, "toolCallId": "8f4d324f-a825-4f68-b5e9-f2dc5b7da6b6", "title": "computer__browser", "toolKind": "computer__browser", "status": "completed", "output": [{"type": "text", "text": "page: NVIDIA Announces CEO Transition | NVIDIA Newsroom\nNewsroom\nNVIDIA in Brief\nExec Bios\nNVIDIA Blog\nPodcast\nMedia Assets\nIn the News\nPress Contacts\nOnline Press Kits\nPress Release\nSHARE \nNVIDIA Announces CEO Transition\nNovember 12, 2026\n…"}], "said": {"key": "browser", "one": "Used the browser", "many": "Used the browser {n} times", "doing": "Browsing"}, "endedTs": 1794536295853},
	{"kind": "tool", "id": "tool:1a4c16ef-ad53-4dd0-93f0-c06e09693630", "ts": 1794536298918, "toolCallId": "1a4c16ef-ad53-4dd0-93f0-c06e09693630", "title": "computer__browser", "toolKind": "computer__browser", "status": "completed", "output": [{"type": "text", "text": "navigated to https://www.cbsnews.com/news/nvidia-ceo-jensen-huang-resigns/"}], "said": {"key": "browser", "one": "Used the browser", "many": "Used the browser {n} times", "doing": "Browsing"}, "endedTs": 1794536299410},
	{"kind": "tool", "id": "tool:36f5ed3b-ec4a-4685-9577-9e9ffae608f0", "ts": 1794536314238, "toolCallId": "36f5ed3b-ec4a-4685-9577-9e9ffae608f0", "title": "computer__browser", "toolKind": "computer__browser", "status": "completed", "output": [{"type": "text", "text": "page: Nvidia CEO Jensen Huang resigns in wake of affair with woman suspected of spying for China - CBS News\nLatest \nLocal News \nLive \nShows \nMoneyWatch\n|\nManaging Your Money\nCBS News Price Tracker\n\nWatch CBS News\n\nMoneyWatch\nNvidia CEO Jens…"}], "said": {"key": "browser", "one": "Used the browser", "many": "Used the browser {n} times", "doing": "Browsing"}, "endedTs": 1794536314261},
	{"kind": "agent", "id": "19bad5be-c325-473a-9b45-3ff1cfea01c3", "ts": 1794536332055, "text": "My take: this is a serious leadership and governance crisis for Nvidia, but it’s too early to treat every allegation as proven—or to conclude Nvidia’s technical advantage disappears with Jensen."},
	{"kind": "agent", "id": "19bad5be-c325-473a-9b45-3ff1cfea01c3-2", "ts": 1794536332055, "text": "[Nvidia’s announcement today](https://nvidianews.nvidia.com/news/nvidia-announces-ceo-transition) confirms that he resigned immediately as CEO and from the board, with CFO Colette Kress taking over as interim CEO. [CBS reports](https://www.cbsnews.com/news/nvidia-ceo-jensen-huang-resigns/) that this follows investigations involving a relationship with a suspected Chinese spy and alleged chip diversion. Those espionage and export-control allegations are distinct from the confirmed resignation; his departure doesn’t establish that he knowingly participated in wrongdoing."},
	{"kind": "agent", "id": "19bad5be-c325-473a-9b45-3ff1cfea01c3-3", "ts": 1794536332055, "text": "What matters most to me isn’t the salacious part. It’s whether personal relationships compromised corporate decisions, access to sensitive technology, or compliance—and what the board knew. Being an exceptional founder doesn’t make someone exempt from accountability. Equally, suspicion isn’t a substitute for evidence."},
	{"kind": "agent", "id": "19bad5be-c325-473a-9b45-3ff1cfea01c3-4", "ts": 1794536332055, "text": "Business-wise, losing him matters: he’s been unusually central to Nvidia’s strategy, customer relationships, and ability to sell a vision. But CUDA, the engineering teams, and customers’ dependence on Nvidia don’t vanish overnight. **I’d be more worried about what an investigation could reveal about the company’s controls than about the absence of Jensen at the next keynote.** The November 17 earnings call should be an important first test of whether the remaining leadership can give concrete answers rather than just reassurance."},
	{"kind": "turn", "id": "d4dab56b-003d-4556-b788-5137754b7b50", "ts": 1794536332058, "stopReason": "end_turn"},
];

const goldieSteps = GOLDIE.filter((event): event is Extract<TranscriptEvent, { kind: "tool" }> => event.kind === "tool");

describe("Goldie's news turn: only the pages it opened are its sources", () => {
	test("the line stacks the two sites it opened, in the order it opened them, and no +N for what the search only listed", async () => {
		for (const site of ["nvidianews.nvidia.com", "cbsnews.com", "bbc.com", "spectrumlocalnews.com"]) icons[site] = icon(site);
		await draw(GOLDIE);
		expect([...stack()!.querySelectorAll("img")].map((img) => img.getAttribute("src"))).toEqual([icon("nvidianews.nvidia.com"), icon("cbsnews.com")]);
		expect(stack()!.querySelector(".favicons-more")).toBeNull();
		expect(stack()!.getAttribute("aria-label")).toBe("Sources: nvidianews.nvidia.com and cbsnews.com");
		await act(async () => line().click());
		const sources = container.querySelector('.steps > section[aria-label="Sources"]')!;
		expect(sources.querySelector(".work-heading")?.textContent).toBe("Sources · 2");
		// Its browser named no title; the search's listing of the same page does.
		expect([...sources.querySelectorAll("a.source-card")].map((card) => [card.querySelector(".source-site")?.textContent, card.querySelector(".source-title")?.textContent])).toEqual([
			["nvidianews.nvidia.com", "NVIDIA Announces CEO Transition"],
			["cbsnews.com", "Nvidia CEO Jensen Huang resigns, board announces - CBS News"],
		]);
		// Every card is a page opened, so none carries the eye.
		expect(sources.querySelector(".source-read")).toBeNull();
	});

	test("in the pane, the search keeps its six results with an eye on the two opened, and the browser's steps open to what they did", async () => {
		const { Trail } = await import("../src/components/transcript/Trail");
		await act(async () => root.render(<Trail items={goldieSteps} settled />));
		expect(container.querySelector('section[aria-label="Sources"] .work-heading')?.textContent).toBe("Sources · 2");
		const lines = [...container.querySelectorAll<HTMLButtonElement>(".trail-head")];
		expect(lines.map((head) => [head.querySelector(".trail-said")?.textContent, head.querySelector(".trail-fact")?.textContent])).toEqual([
			["Searched the web", "6 results"],
			["Read SKILL.md", expect.any(String)],
			["Used the computer 2 times", expect.any(String)],
			["Used the browser 4 times", expect.any(String)],
		]);
		await act(async () => lines[0]!.click());
		await act(async () => container.querySelector<HTMLButtonElement>(".trail-search .trail-row")!.click());
		const hits = [...container.querySelectorAll(".trail-hit")];
		expect(hits).toHaveLength(6);
		expect(hits.filter((hit) => hit.querySelector(".source-read") !== null).map((hit) => hit.getAttribute("href"))).toEqual([
			"https://www.cbsnews.com/news/nvidia-ceo-jensen-huang-resigns/",
			"https://nvidianews.nvidia.com/news/nvidia-announces-ceo-transition",
		]);
		await act(async () => lines[3]!.click());
		const browsing = lines[3]!.closest(".trail-step")!;
		expect(browsing.querySelectorAll(".trail-run")).toHaveLength(4);
		expect(browsing.querySelector(".trail-page")).toBeNull();
		expect(browsing.querySelector(".trail-out")?.textContent).toBe("navigated to https://nvidianews.nvidia.com/news/nvidia-announces-ceo-transition");
	});

	test("the same question searched and never opened has no sources: no stack, no Sources group, and the search still says 6 results", async () => {
		const searchedOnly = GOLDIE.filter((event) => event.kind !== "tool" || event.title.startsWith("web_search"));
		await draw(searchedOnly);
		expect(stack()).toBeNull();
		await act(async () => line().click());
		expect(container.querySelector('.steps [aria-label="Sources"]')).toBeNull();
		const { Trail } = await import("../src/components/transcript/Trail");
		await act(async () => root.render(<Trail items={goldieSteps.filter((step) => step.title.startsWith("web_search"))} settled />));
		expect(container.querySelector('[aria-label="Sources"]')).toBeNull();
		expect(container.querySelector(".trail-head .trail-fact")?.textContent).toBe("6 results");
	});
});
