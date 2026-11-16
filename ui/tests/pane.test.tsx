import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import { clockWords, threadFamily } from "../src/threadKinds";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { BandKind, PaneHead, TaskCard, LiveStream, ResultCard } = await import("../src/components/thread/Pane");
const { Clamped, SectionRule, StateLine } = await import("../src/components/thread/Spine");

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

describe("a thread's kind", () => {
	test("a work thread the person is in is a conversation, and one handed over is a handoff they can step in to", () => {
		expect(threadFamily("side")).toEqual({ glyph: "≡", word: "Work thread", watching: false });
		expect(threadFamily("side", "Elvis")).toEqual({ glyph: "⇄", word: "Handoff", watching: true });
	});

	test("a run is a helper unless it is a command a teammate left running", () => {
		expect(threadFamily("run")).toEqual({ glyph: "◆", word: "Helper", watching: true });
		expect(threadFamily("run", undefined, "shell")).toEqual({ glyph: "$", word: "Command", watching: true });
	});

	test("a pair and a job are watched, a call is one the person is in", () => {
		expect(threadFamily("pair")).toEqual({ glyph: "◎", word: "Conversation", watching: true });
		expect(threadFamily("job")).toEqual({ glyph: "↻", word: "Job", watching: true });
		expect(threadFamily("call")).toEqual({ glyph: "☏", word: "Call", watching: false });
	});

	test("a time today is its time, and an earlier one has its date", () => {
		const now = new Date(2026, 10, 7, 14, 0).getTime();
		expect(clockWords(new Date(2026, 10, 7, 13, 57).getTime(), now)).toMatch(/^1:57\s?PM$/);
		expect(clockWords(new Date(2026, 10, 3, 9, 5).getTime(), now)).toMatch(/^Nov 3, 9:05\s?AM$/);
	});
});

describe("a pane's band", () => {
	test("no kind icon in the top bar: the kind's word alone, its state kept for a reader", async () => {
		for (const kind of [threadFamily("run"), threadFamily("run", undefined, "shell"), threadFamily("job"), threadFamily("call"), threadFamily("pair"), threadFamily("side", "Elvis")]) {
			await render(<BandKind kind={kind} state="working" />);
			const heading = container.querySelector("h2");
			expect(heading?.textContent).toBe(kind.word);
			expect(heading?.textContent).not.toContain(kind.glyph);
			expect(heading?.querySelector(".run-dot, [aria-hidden]")).toBeNull();
			expect(heading?.getAttribute("aria-label")).toBe(`${kind.word}, working`);
		}
	});
});

describe("a pane's head", () => {
	test("says the title on the text edge, then one state line: its word, a detail, and a time at the right edge, with no dot", async () => {
		await render(<PaneHead title="Audit side-pane thread rows" line={<StateLine word="Working now" detail="6 steps" time="5:27" />} />);
		expect(container.querySelector(".spine-title")?.textContent).toBe("Audit side-pane thread rows");
		const line = container.querySelector(".spine-state");
		expect(line?.querySelector(".spine-state-word")?.textContent).toBe("Working now");
		expect(line?.querySelector(".spine-state-detail")?.textContent).toBe("· 6 steps");
		expect(line?.querySelector(".spine-time")?.textContent).toBe("5:27");
		expect(container.querySelector(".spine-mark")).toBeNull();
		// Spacing closes the head, not a hairline.
		expect(container.querySelector("hr, .pane-head")).toBeNull();
	});

	test("waiting on the person, the state's word is the amber one", async () => {
		await render(<PaneHead title="Sign the installer" line={<StateLine word="Needs you" time="11:39 AM" you />} />);
		expect(container.querySelector(".spine-state")?.hasAttribute("data-you")).toBe(true);
	});

	test("its facts are rows of a label and a value, empty ones left out, at most three", async () => {
		await render(<PaneHead title="Profile the cache" line={<StateLine word="Done" />} facts={[["Sent by", "Huck"], ["Model", undefined], ["Started", "11:36 AM"]]} />);
		const rows = [...container.querySelectorAll(".spine-facts > div")].map((row) => [row.querySelector("dt")?.textContent, row.querySelector("dd")?.textContent]);
		expect(rows).toEqual([
			["Sent by", "Huck"],
			["Started", "11:36 AM"],
		]);
		// @ts-expect-error A fourth fact is refused: the title and the state line say the rest.
		await render(<PaneHead title="Too many" line={null} facts={[["A", "1"], ["B", "2"], ["C", "3"], ["D", "4"]]} />);
	});
});

describe("a section's heading", () => {
	test("sits on a rule with its count at the rule's end, in sentence case", async () => {
		await render(<SectionRule title="Receipts" count={4} />);
		const rule = container.querySelector("h4.spine-rule");
		expect(rule?.querySelector(".spine-rule-title")?.textContent).toBe("Receipts");
		expect(rule?.querySelector(".spine-rule-line")).not.toBeNull();
		expect(rule?.querySelector(".spine-rule-count")?.textContent).toBe("4");
		expect(rule?.querySelector(".spine-rule-mark")).toBeNull();
	});

	test("the status mark's four states: mint while active, an amber ring when blocked on the person, a dim dot once stopped, a red cross when it failed", async () => {
		const drawn = { live: ["live", false], blocked: ["waiting", true], stopped: ["stopped", false], failed: ["failed", false] } as const;
		for (const [status, [mark, blocked]] of Object.entries(drawn)) {
			await render(<SectionRule title="Activity" status={status as keyof typeof drawn} />);
			const rule = container.querySelector(".spine-rule");
			expect(rule?.getAttribute("data-status")).toBe(status);
			const dot = rule?.querySelector(".spine-rule-mark > .spine-mark");
			expect(dot?.getAttribute("data-mark")).toBe(mark);
			expect(dot?.hasAttribute("data-blocked")).toBe(blocked);
			expect(dot?.classList.contains("beat")).toBe(status === "live");
			expect(rule?.querySelectorAll(".spine-mark")).toHaveLength(1);
		}
		const words = [];
		for (const status of ["live", "blocked", "stopped", "failed"] as const) {
			await render(<SectionRule title="Activity" status={status} />);
			words.push(container.querySelector(".spine-rule-mark")?.getAttribute("aria-label"));
		}
		expect(words).toEqual(["Active", "Needs you", "Ended", "Failed"]);
	});
});

describe("words someone wrote", () => {
	const more = () => [...container.querySelectorAll("button")].find((one) => one.textContent === "Show all" || one.textContent === "Show less");

	test("a task names its section and who it came from, with their face, at the rule's end, and reads as markdown", async () => {
		await render(<TaskCard eyebrow="Request" from={{ personaId: "pc", name: "Pork Chop" }} text={"Check these:\n\n- the numbers\n- the links"} />);
		expect(container.querySelector(".pane-task .spine-rule-title")?.textContent).toBe("Request");
		expect(container.querySelector(".spine-rule .pane-task-from")?.textContent).toEndWith("Pork Chop");
		expect(container.querySelector(".pane-task-from .avatar")).not.toBeNull();
		expect([...container.querySelectorAll(".spine-said li")].map((li) => li.textContent)).toEqual(["the numbers", "the links"]);
	});

	test("has no Show all when its words fit in six lines", async () => {
		measured = { scrollHeight: 132, clientHeight: 132 };
		await render(<TaskCard eyebrow="Request" text="Short." />);
		expect(more()).toBeUndefined();
	});

	test("the six-line clamp: past six lines Show all opens it in place and Show less folds it again", async () => {
		measured = { scrollHeight: 400, clientHeight: 132 };
		await render(<TaskCard eyebrow="Asked by Huck" text="A long task." />);
		const body = () => container.querySelector(".spine-clamp");
		expect(body()?.hasAttribute("data-open")).toBe(false);
		expect(body()?.hasAttribute("data-clipped")).toBe(true);
		expect(more()?.textContent).toBe("Show all");
		expect(more()?.getAttribute("aria-expanded")).toBe("false");

		await act(async () => more()?.click());
		expect(body()?.hasAttribute("data-open")).toBe(true);
		expect(body()?.hasAttribute("data-clipped")).toBe(false);
		expect(more()?.textContent).toBe("Show less");

		await act(async () => more()?.click());
		expect(body()?.hasAttribute("data-open")).toBe(false);
		expect(more()?.textContent).toBe("Show all");
	});

	test("the clamp is six lines of the reading size, no fewer and no more", async () => {
		await render(<TaskCard eyebrow="Request" text="Words." />);
		expect((container.querySelector(".spine-clamp") as HTMLElement | null)?.style.getPropertyValue("--clamp-lines")).toBe("6");
		const css = await Bun.file(new URL("../src/pane-spine.css", import.meta.url)).text();
		expect(css).toContain("max-height: calc(var(--clamp-lines) * var(--text-read--line-height));");
	});

	test("any clamped words keep their own key: two on a page open one at a time", async () => {
		measured = { scrollHeight: 400, clientHeight: 132 };
		await render(
			<>
				<Clamped watch="a">one</Clamped>
				<Clamped watch="b">two</Clamped>
			</>,
		);
		const keys = () => [...container.querySelectorAll(".spine-more")];
		await act(async () => (keys()[1] as HTMLButtonElement).click());
		expect(keys().map((key) => key.textContent)).toEqual(["Show all", "Show less"]);
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

describe("a result", () => {
	test("closes the stream as a section, how long at the rule's end, and its words as markdown on a rule", async () => {
		await render(<ResultCard eyebrow="Result" aside="3:34" text={"A run is shown on **6 surfaces**.\n\n- Add `lastStep`"} />);
		const card = container.querySelector(".pane-result");
		expect(card?.getAttribute("aria-label")).toBe("Result");
		expect(card?.querySelector(".spine-rule-title")?.textContent).toBe("Result");
		expect(card?.querySelector(".spine-rule-count")?.textContent).toBe("3:34");
		expect(card?.querySelector(".spine-said strong")?.textContent).toBe("6 surfaces");
		expect(card?.querySelector(".spine-said li code")?.textContent).toBe("lastStep");
	});
});
