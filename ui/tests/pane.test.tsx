import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import { startedWords, threadFamily } from "../src/threadKinds";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { PaneHead, TaskCard, LiveStream, ResultCard } = await import("../src/components/thread/Pane");

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
/** What every element measures, so a test can say whether the task's lines overflow three. */
let measured: { scrollHeight: number; clientHeight: number };

beforeEach(async () => {
	measured = { scrollHeight: 0, clientHeight: 0 };
	const installed = installDom();
	teardown = installed.teardown;
	const proto = installed.dom.HTMLElement.prototype;
	Object.defineProperty(proto, "scrollHeight", { configurable: true, get: () => measured.scrollHeight });
	Object.defineProperty(proto, "clientHeight", { configurable: true, get: () => measured.clientHeight });
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

async function render(node: React.ReactNode) {
	await act(async () => root.render(node));
}

describe("a thread's kind and family", () => {
	test("a work thread the person is in is a conversation, and one handed over is a handoff they can step in to", () => {
		expect(threadFamily("side")).toEqual({ glyph: "≡", word: "Work thread", family: "", watching: false });
		expect(threadFamily("side", "Elvis")).toEqual({ glyph: "⇄", word: "Handoff", family: "", watching: true });
	});

	test("a run is a subagent, read only, unless it is a command a teammate left running", () => {
		expect(threadFamily("run", "Pork Chop")).toEqual({ glyph: "◆", word: "Subagent", family: "Read only", watching: true });
		expect(threadFamily("run", "Pork Chop", "shell")).toEqual({ glyph: "$", word: "Command", family: "Started by Pork Chop", watching: true });
		expect(threadFamily("run", undefined, "shell").family).toBe("");
	});

	test("a pair and a job are watched, a call is one the person is in", () => {
		expect(threadFamily("pair")).toMatchObject({ glyph: "◎", word: "Pair", family: "Read only", watching: true });
		expect(threadFamily("job")).toMatchObject({ glyph: "↻", word: "Job", watching: true });
		expect(threadFamily("call")).toMatchObject({ glyph: "☏", word: "Call", family: "", watching: false });
	});

	test("a start today is its time, and an earlier one has its date", () => {
		const now = new Date(2026, 10, 7, 14, 0).getTime();
		expect(startedWords(new Date(2026, 10, 7, 13, 57).getTime(), now)).toMatch(/^started 1:57\s?PM$/);
		expect(startedWords(new Date(2026, 10, 3, 9, 5).getTime(), now)).toMatch(/^started Nov 3, 9:05\s?AM$/);
	});
});

describe("a pane's head", () => {
	test("says the title, then the kind's fact and the meta joined by dots without empty parts", async () => {
		await render(<PaneHead kind={threadFamily("run", "Pork Chop")} title="Audit side-pane thread rows" meta={["sent by Pork Chop", "", undefined, false, "Sonnet 5.5", "started 1:57 PM"]} />);
		expect(container.querySelector(".pane-title")?.textContent).toBe("Audit side-pane thread rows");
		expect(container.querySelector(".pane-meta")?.textContent).toBe("Read only · sent by Pork Chop · Sonnet 5.5 · started 1:57 PM");
	});

	test("a thread the person speaks in has no fact leading its meta: its composer says it", async () => {
		await render(<PaneHead kind={threadFamily("side")} title="Release notes" meta={["Pork Chop"]} />);
		expect(container.querySelector(".pane-title")?.textContent).toBe("Release notes");
		expect(container.querySelector(".pane-meta")?.textContent).toBe("Pork Chop");
	});

	test("a family said in its place wins over the kind's own", async () => {
		await render(<PaneHead kind={threadFamily("job")} family="Loop" title="Watch CI" />);
		expect(container.querySelector(".pane-meta")?.textContent).toBe("Loop");
	});
});

describe("a task card", () => {
	const task = "Read Dock.tsx, Conversation.tsx and LinkRow.tsx. List every place a subagent's run is shown.";
	const more = () => [...container.querySelectorAll("button")].find((one) => one.textContent === "Show all" || one.textContent === "Show less");

	test("names its eyebrow and who it came from, with their face", async () => {
		await render(<TaskCard eyebrow="Task" from={{ personaId: "pc", name: "Pork Chop" }} text={task} />);
		expect(container.querySelector(".pane-task-head .eyebrow")?.textContent).toBe("Task");
		expect(container.querySelector(".pane-task-from")?.textContent).toContain("from Pork Chop");
		expect(container.querySelector(".pane-task-from .avatar")).not.toBeNull();
		expect(container.querySelector(".pane-task-body")?.textContent).toBe(task);
	});

	test("has no Show all when its words fit in three lines", async () => {
		measured = { scrollHeight: 54, clientHeight: 54 };
		await render(<TaskCard eyebrow="Request" text="Short." />);
		expect(more()).toBeUndefined();
	});

	test("clamped past three lines, opens with Show all and closes with Show less", async () => {
		measured = { scrollHeight: 160, clientHeight: 54 };
		await render(<TaskCard eyebrow="Task" text={task} />);
		const body = () => container.querySelector(".pane-task-body");
		expect(body()?.hasAttribute("data-open")).toBe(false);
		expect(more()?.textContent).toBe("Show all");
		expect(more()?.getAttribute("aria-expanded")).toBe("false");

		await act(async () => more()?.click());
		expect(body()?.hasAttribute("data-open")).toBe(true);
		expect(more()?.textContent).toBe("Show less");

		await act(async () => more()?.click());
		expect(body()?.hasAttribute("data-open")).toBe(false);
		expect(more()?.textContent).toBe("Show all");
	});
});

describe("a live stream", () => {
	test("follows the newest, and offers Latest once scrolled up", async () => {
		await render(
			<LiveStream following="run-1" count={2} content={2} live>
				<p>Read Dock.tsx</p>
				<p>Search "subagent"</p>
			</LiveStream>,
		);
		const frame = container.querySelector<HTMLElement>(".pane-stream");
		expect(frame?.textContent).toContain("Read Dock.tsx");
		const latest = () => [...container.querySelectorAll("button")].find((one) => one.textContent === "Latest");
		expect(latest()).toBeUndefined();

		// Scrolled up: the newest is out of view.
		measured = { scrollHeight: 900, clientHeight: 300 };
		await act(async () => {
			frame?.dispatchEvent(new window.Event("scroll", { bubbles: true }) as unknown as Event);
		});
		expect(latest()).toBeDefined();
		expect(latest()?.querySelector(".beat")).not.toBeNull();
	});
});

describe("a result card", () => {
	test("closes the stream with its eyebrow and its words as markdown", async () => {
		await render(<ResultCard eyebrow="Result · sent to Pork Chop" aside="3:34" text={"A run is shown on **6 surfaces**.\n\n- Add `lastStep`"} />);
		const card = container.querySelector(".pane-result");
		expect(card?.getAttribute("aria-label")).toBe("Result · sent to Pork Chop");
		expect(card?.querySelector(".pane-result-head")?.textContent).toBe("Result · sent to Pork Chop3:34");
		expect(card?.querySelector("strong")?.textContent).toBe("6 surfaces");
		expect(card?.querySelector("li code")?.textContent).toBe("lastStep");
	});
});
