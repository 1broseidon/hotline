import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { TranscriptEvent } from "../src/generated/contract";
import type { Step } from "../src/scheduledRuns";
import { installDom } from "./dom";

// Outcomes, not work (epic-42): the teammate says what it is on and what came
// of it, and the desk draws those words and never makes its own.

const user = (id: string, ts: number): TranscriptEvent => ({ kind: "user", id, ts, text: "fill in the aftercare form" });
const turnEnd = (id: string, ts: number): TranscriptEvent => ({ kind: "turn", id, ts, stopReason: "end_turn" });
const tool = (id: string, ts: number, toolKind: string, said?: string): Extract<TranscriptEvent, { kind: "tool" }> => ({
	kind: "tool",
	id,
	ts,
	endedTs: ts + 500,
	toolCallId: id,
	title: `step ${id}`,
	toolKind,
	status: "completed",
	...(said !== undefined ? { said: { key: said, one: said, many: said, doing: said } } : {}),
});
const doing = (id: string, ts: number, type: string, message?: string): TranscriptEvent => ({ kind: "activity", id, ts, type, ...(message !== undefined ? { message } : {}) });
const left = (id: string, ts: number, type: string, message: string, more: object = {}): TranscriptEvent => ({ kind: "outcome", id, ts, type, message, key: id, ...more });

describe("what the mark says while a turn runs", () => {
	test("each declared kind moves the mark its own way, its words beside it; a kind the desk does not know moves it plainly", async () => {
		const { activityOf } = await import("../src/activity");
		const said = (type: string) => activityOf([user("u", 1), doing("d", 2, type, "Filling out form T-128")], []);
		expect(["thinking", "reading", "searching", "making", "talking", "juggling"].map((type) => said(type).phase)).toEqual([
			"thinking",
			"read",
			"search",
			"edit",
			"talking",
			"doing",
		]);
		expect(said("making").word).toBe("Filling out form T-128");
	});

	test("the newest declaration stands over the tools that follow it, and the calls that declare are not work", async () => {
		const { activityOf } = await import("../src/activity");
		const events = [user("u", 1), doing("d1", 2, "reading", "Reading the lease"), tool("t1", 3, "read"), doing("d2", 4, "searching", "Finding plumbers near you"), tool("t2", 5, "execute"), tool("t3", 6, "other", "activity")];
		expect(activityOf(events, [])).toEqual({ phase: "search", word: "Finding plumbers near you" });
	});

	test("with nothing declared the tool may still move the mark, and nothing is said: no Working, no tool's words", async () => {
		const { activityOf } = await import("../src/activity");
		expect(activityOf([user("u", 1), tool("t1", 2, "execute", "Running cargo test")], [])).toEqual({ phase: "execute", word: "" });
		expect(activityOf([user("u", 1)], [])).toEqual({ phase: "thinking", word: "" });
	});

	test("needs you and the reply on its way stay the desk's own, over any declaration", async () => {
		const { activityOf } = await import("../src/activity");
		const declared = [user("u", 1), doing("d", 2, "talking", "Asking Huck for the numbers")];
		const card: TranscriptEvent = { kind: "permission", id: "p", ts: 3, requestId: "r", title: "Allow?", options: [] };
		expect(activityOf([...declared, card], []).phase).toBe("blocked");
		expect(activityOf(declared, [{ messageId: "a", kind: "agent", ts: 4, text: "Huck has" }])).toEqual({ phase: "writing", word: "" });
	});

	test("a declaration is cleared when its turn ends: the next turn starts with nothing said", async () => {
		const { activityOf } = await import("../src/activity");
		const events = [user("u1", 1), doing("d", 2, "reading", "Reading the lease"), turnEnd("e", 3), user("u2", 4)];
		expect(activityOf(events, [])).toEqual({ phase: "thinking", word: "" });
	});
});

describe("what the mark says while the turn waits on its subagents", () => {
	const reply: TranscriptEvent = { kind: "agent", id: "a", ts: 5, text: "The helpers are out; I'll build the chart when they report." };
	const helper = tool("t1", 3, "other", "subagent");

	test("it rests, with the words the teammate last said it is on in this turn", async () => {
		const { waitingOf } = await import("../src/activity");
		const events = [user("u", 1), doing("d1", 2, "searching", "Scanning the field"), helper, doing("d2", 4, "searching", "Waiting on four feature checks"), reply];
		expect(waitingOf(events)).toEqual({ phase: "rest", word: "Waiting on four feature checks" });
	});

	test("with nothing declared it rests alone, and an earlier turn's words do not carry over", async () => {
		const { waitingOf } = await import("../src/activity");
		expect(waitingOf([user("u", 1), helper, reply])).toEqual({ phase: "rest", word: "" });
		expect(waitingOf([user("u0", 0), doing("d", 0.5, "reading", "Reading the lease"), turnEnd("e", 0.7), user("u", 1), helper, reply])).toEqual({ phase: "rest", word: "" });
	});

	test("a waiting turn reads as done everywhere but the mark: no Interrupt, no beat, no working", async () => {
		const { awaitingSubagents, isWorking, shownState } = await import("../src/activity");
		const waiting = { state: "thinking" as const, awaitingSubagents: true };
		expect([awaitingSubagents(waiting), isWorking(waiting), shownState(waiting)]).toEqual([true, false, "ready"]);
		// The core sets it only mid-turn; a stale one never rests the mark of a stopped teammate.
		expect(awaitingSubagents({ state: "stopped", awaitingSubagents: true })).toBe(false);
		expect(awaitingSubagents({ state: "thinking" })).toBe(false);
	});
});

describe("a turn's path", () => {
	test("what the teammate said rides with the turn's steps, and the calls that said it are not steps", async () => {
		const { toBlocks } = await import("../src/transcriptFolds");
		const blocks = toBlocks([user("u", 1), doing("d", 2, "reading", "Reading the lease"), tool("t1", 3, "other", "activity"), tool("t2", 4, "read"), left("o", 5, "made", "Made a summary"), { kind: "agent", id: "a", ts: 6, text: "Done." }, turnEnd("e", 7)]);
		const steps = blocks.find((block) => block.kind === "steps");
		expect(steps?.kind === "steps" && steps.items.map((item) => item.id)).toEqual(["d", "t2", "o"]);
	});

	test("steps group under the activity they happened during, those before any come first, and outcomes stand where they were said", async () => {
		const { pathGroups } = await import("../src/outcomes");
		const items = [
			tool("t0", 1, "read", "Read a file"),
			doing("d1", 4, "reading", "Checking the form"),
			tool("t1", 5, "read", "Read 2 documents"),
			left("o1", 6, "sent", "Sent Huck a message"),
			doing("d2", 7, "making"),
			tool("t2", 8, "edit", "Edited a file"),
			doing("d3", 9, "making", "Filling in the form"),
			tool("t3", 10, "edit", "Edited the form"),
		] as Step[];
		const shape = pathGroups(items, false).map((group) => [
			group.heading?.message ?? null,
			group.entries.map((entry) => (entry.kind === "line" ? entry.line.text : `outcome:${entry.outcome.message}`)),
		]);
		expect(shape).toEqual([
			[null, ["Read a file"]],
			["Checking the form", ["Read 2 documents", "outcome:Sent Huck a message"]],
			// Said with no words, it heads nothing: the desk does not make words up.
			[null, ["Edited a file"]],
			["Filling in the form", ["Edited the form"]],
		]);
	});

	test("while it runs, the last line says what is happening now and a fresh heading stands alone", async () => {
		const { pathGroups } = await import("../src/outcomes");
		const running = pathGroups([tool("t1", 1, "read", "Read a file"), doing("d1", 2, "searching", "Finding the kids' details")] as Step[], true);
		expect(running.map((group) => [group.heading?.message ?? null, group.entries.length])).toEqual([
			[null, 1],
			["Finding the kids' details", 0],
		]);
	});
});

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

describe("drawn", () => {
	beforeEach(async () => {
		// The transcript watches its column's size; nothing here lays out, so nothing changes size.
		const ResizeObserver = class {
			observe() {}
			disconnect() {}
		};
		({ teardown } = installDom({ requestAnimationFrame: () => 0, cancelAnimationFrame: () => {}, ResizeObserver }));
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

	test("the pane heads its steps with the teammate's words and when each began, set in under it, and marks each outcome", async () => {
		const { Trail } = await import("../src/components/transcript/Trail");
		const items = [
			tool("t0", 1_000, "read", "Read a file"),
			doing("d1", 5_000, "reading", "Checking the form"),
			tool("t1", 6_000, "read", "Read 2 documents"),
			doing("d2", 71_000, "making", "Filling in the form"),
			left("o1", 80_000, "made", "Aftercare form, 18 of 22 answers", { state: "partial" }),
		] as Step[];
		await act(async () => root.render(<Trail items={items} settled />));
		const top = container.querySelector("section[aria-label='Steps'] > ol.trail")!;
		expect([...top.children].map((one) => one.className)).toEqual(["trail-step", "trail-group", "trail-group"]);
		const headings = [...container.querySelectorAll(".trail-heading")].map((one) => [one.querySelector(".trail-heading-said")?.textContent, one.querySelector(".trail-fact")?.textContent]);
		expect(headings).toEqual([
			["Checking the form", "0:04"],
			["Filling in the form", "1:10"],
		]);
		const under = [...container.querySelectorAll(".trail-group")].map((group) => [...group.querySelectorAll(".trail-under > li")].map((one) => one.textContent));
		expect(under[0]).toEqual([expect.stringContaining("Read 2 documents")]);
		const outcome = container.querySelector(".trail-group .trail-outcome")!;
		expect(outcome.getAttribute("data-mark")).toBe("partial");
		expect(outcome.textContent).toBe("Aftercare form, 18 of 22 answers");
		expect(container.querySelector(".work-heading.steps-heading")?.textContent).toBe("Steps · 2");
	});

	test("waiting on its subagents the mark stays in the composer at rest with the teammate's words, and goes to sleep only when the turn ends", async () => {
		const { Transcript } = await import("../src/components/Transcript");
		const marks: (import("../src/activity").WorkMark | null)[] = [];
		const declared = [user("u", 1), doing("d", 2, "searching", "Waiting on four feature checks"), tool("t1", 3, "other", "subagent"), { kind: "agent" as const, id: "a", ts: 4, text: "The helpers are out." }];
		const draw = (events: TranscriptEvent[], live: boolean, waiting: boolean) =>
			act(async () => root.render(<Transcript personaId="quill" name="Quill" events={events} streaming={[]} live={live} waiting={waiting} focus={null} onMark={(mark) => marks.push(mark)} />));
		await draw(declared, false, true);
		expect(marks.at(-1)).toEqual({ activity: { phase: "rest", word: "Waiting on four feature checks" }, sleeping: false });
		// A helper reports and the turn works again: the mark moves by what it said.
		await draw(declared, true, false);
		expect(marks.at(-1)).toEqual({ activity: { phase: "search", word: "Waiting on four feature checks" }, sleeping: false });
		// Truly idle: the turn has ended, the words go and the mark sinks back to sleep.
		await draw([...declared, turnEnd("e", 6)], false, false);
		expect(marks.at(-1)).toEqual({ activity: { phase: "rest", word: "" }, sleeping: true });
	});

	test("waiting with nothing declared, the toad rests alone; the composer offers no Interrupt and Escape stops nothing", async () => {
		const { Transcript } = await import("../src/components/Transcript");
		const { Composer } = await import("../src/components/Composer");
		const { awaitingSubagents, shownState } = await import("../src/activity");
		const marks: (import("../src/activity").WorkMark | null)[] = [];
		await act(async () =>
			root.render(<Transcript personaId="quill" name="Quill" events={[user("u", 1), tool("t1", 2, "other", "subagent")]} streaming={[]} live={false} waiting focus={null} onMark={(mark) => marks.push(mark)} />),
		);
		const mark = marks.at(-1)!;
		expect(mark).toEqual({ activity: { phase: "rest", word: "" }, sleeping: false });
		const session = { state: "thinking" as const, awaitingSubagents: true };
		expect(awaitingSubagents(session)).toBe(true);
		const cancelled: string[] = [];
		const composer = (state: import("../src/generated/contract").SessionState) =>
			act(async () =>
				root.render(
					<Composer
						personaId="quill"
						name="Quill"
						state={state}
						replyQuote={null}
						onSend={() => {}}
						onCancel={() => cancelled.push(state)}
						onClearReply={() => {}}
						work={{ ...mark, open: false, onToggle: () => {} }}
					/>,
				),
			);
		const pressEscape = () => act(async () => void container.querySelector("textarea")!.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
		await composer(shownState(session));
		const slot = container.querySelector<HTMLButtonElement>("button.composer-work")!;
		expect(slot.querySelector(".glyph")).not.toBeNull();
		expect(slot.hasAttribute("data-sleeping")).toBe(false);
		expect(slot.textContent).toBe("");
		expect(container.querySelector('[aria-label="Interrupt"]')).toBeNull();
		await pressEscape();
		expect(cancelled).toEqual([]);
		// The same key in a turn that works is an Interrupt: the quiet above is the wait's, not the field's.
		await composer("thinking");
		expect(container.querySelector('[aria-label="Interrupt"]')).not.toBeNull();
		await pressEscape();
		expect(cancelled).toEqual(["thinking"]);
	});

	test("a reply said just before the wait hangs up first, keeping the words, then rests", async () => {
		const { Transcript } = await import("../src/components/Transcript");
		const { LANDED_MS } = await import("../src/ui/Glyph");
		const marks: (import("../src/activity").WorkMark | null)[] = [];
		const events = [user("u", 1), doing("d", 2, "searching", "Waiting on four feature checks"), tool("t1", 3, "other", "subagent")];
		const draw = (live: boolean, waiting: boolean, streaming: import("../src/tape").Streaming[]) =>
			act(async () => root.render(<Transcript personaId="quill" name="Quill" events={events} streaming={streaming} live={live} waiting={waiting} focus={null} onMark={(mark) => marks.push(mark)} />));
		await draw(true, false, [{ messageId: "a", kind: "agent", ts: 4, text: "The helpers are out." }]);
		expect(marks.at(-1)?.activity.phase).toBe("writing");
		await draw(false, true, []);
		expect(marks.at(-1)).toEqual({ activity: { phase: "landed", word: "Waiting on four feature checks" }, sleeping: false });
		await act(async () => void (await new Promise((resolve) => setTimeout(resolve, LANDED_MS + 50))));
		expect(marks.at(-1)).toEqual({ activity: { phase: "rest", word: "Waiting on four feature checks" }, sleeping: false });
	});

	test("talking puts the headset on: the brow fades into a band with cups and a mic, and comes off on leaving", async () => {
		const { Glyph } = await import("../src/ui/Glyph");
		const opacity = (part: string) => container.querySelector(part)?.getAttribute("opacity");
		const until = async (seen: () => boolean, what: string) => {
			for (let turn = 0; turn < 120 && !seen(); turn++) await act(async () => void (await new Promise((resolve) => setTimeout(resolve, 25))));
			if (!seen()) throw new Error(`Never drawn: ${what}`);
		};
		await act(async () => root.render(<Glyph phase="talking" />));
		// The brow's ends drop to the ears and its middle rises into a band, once.
		const band = () => container.querySelector(".g-band")?.getAttribute("d");
		await until(() => band() === "M4.8 24.00C4.8 2.00 59.2 2.00 59.2 24.00", "the headset on");
		expect(opacity(".g-hp")).toBe("1.00");
		expect(opacity(".g-mic")).toBe("1.00");
		expect(opacity(".g-hand")).toBe("0.00");
		await act(async () => root.render(<Glyph phase="read" />));
		await until(() => opacity(".g-hp") === "0.00", "the headset off");
		expect(opacity(".g-hand")).toBe("1.00");
	});
});
