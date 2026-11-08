import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });

let act: typeof import("react").act;
let CallPage: typeof import("../src/components/thread/CallPane").CallPage;
let callClock: typeof import("../src/components/thread/CallPane").callClock;
let hangUpOf: typeof import("../src/components/thread/CallPane").hangUpOf;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

beforeEach(async () => {
	const installed = installDom({ requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	Object.assign(globalThis, { ResizeObserver: installed.dom.ResizeObserver });
	act = (await import("react")).act;
	({ CallPage, callClock, hangUpOf } = await import("../src/components/thread/CallPane"));
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

const NOW = 1_800_000_000_000;
const said = (key: string, one: string) => ({ key, one, many: `${one} ({n})`, doing: one });

const transcript: TranscriptEvent[] = [
	{ kind: "user", id: "u1", ts: 1, text: "Is the old token still working for live calls?" },
	{ kind: "agent", id: "a1", ts: 2, text: "Let me check the log for which login this call used." },
	{ kind: "tool", id: "t1", ts: 3, toolCallId: "tc1", title: 'grep "[voice] call" hotline.log', status: "completed", said: said("search", "Searched the voice log") },
	{ kind: "tool", id: "t2", ts: 4, toolCallId: "tc2", title: "Read vault/logins", status: "completed", said: said("read", "Read the logins") },
	{ kind: "agent", id: "a2", ts: 5, text: "This call is on the device-code login, and it's holding." },
];

type Props = Partial<Parameters<typeof CallPage>[0]>;
let ended: number;
let opened: import("../src/components/Transcript").ThreadRef[];

async function draw(props: Props = {}) {
	ended = 0;
	opened = [];
	await act(async () =>
		root.render(
			<CallPage
				thread={{ kind: "call", key: "c1" }}
				title="Live call on device-code login test"
				personaId="pork"
				name="Pork Chop"
				live
				startedAt={NOW - 768_000}
				model="GPT-Live"
				events={transcript}
				working={false}
				now={NOW}
				onEnd={() => (ended += 1)}
				onOpenThread={(open) => opened.push(open)}
				onBack={() => {}}
				onClose={() => {}}
				composer={<textarea aria-label="Message" />}
				{...props}
			/>,
		),
	);
}

const text = (selector: string) => [...container.querySelectorAll(selector)].map((el) => el.textContent);
const button = (words: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find((one) => one.textContent === words);

describe("a call's pane", () => {
	test("says ☏ Call with no family line, and the band says Live with how long and the voice model, and End call while live", async () => {
		await draw();
		expect(container.querySelector(".pane-kind")?.textContent).toBe("☏Call");
		expect(container.querySelector(".pane-family")).toBeNull();
		expect(container.querySelector(".pane-title")?.textContent).toBe("Live call on device-code login test");
		expect(container.querySelector(".band h2")?.textContent).toBe("Live");
		expect(container.querySelector(".band .instrument")?.textContent).toBe("12:48 · GPT-Live");
		await act(async () => button("End call")?.click());
		expect(ended).toBe(1);
	});

	test("ended, the band says Ended and how long it ran, with no End call and no composer", async () => {
		await draw({ live: false, endedAt: NOW - 60_000, model: undefined, composer: <textarea aria-label="Message" /> });
		expect(container.querySelector(".band h2")?.textContent).toBe("Ended");
		expect(container.querySelector(".band .instrument")?.textContent).toBe("11:48");
		expect(button("End call")).toBeUndefined();
		expect(container.querySelector('[aria-label="Message"]')).toBeNull();
	});

	test("hung up with its thread still open, the band says Ended and the composer stays, as today", async () => {
		await draw({ live: false, open: true, endedAt: NOW - 60_000, model: undefined });
		expect(container.querySelector(".band h2")?.textContent).toBe("Ended");
		expect(button("End call")).toBeUndefined();
		expect(container.querySelector('[aria-label="Message"]')).not.toBeNull();
	});

	test("lines are labelled by speaker, You or the teammate, in a two-column transcript", async () => {
		await draw();
		expect(text(".call-line .call-who")).toEqual(["You", "Pork Chop", "Pork Chop"]);
		expect(container.querySelector(".call-line")?.hasAttribute("data-mine")).toBe(true);
		expect(text(".call-line .call-words")).toEqual([
			"Is the old token still working for live calls?",
			"Let me check the log for which login this call used.",
			"This call is on the device-code login, and it's holding.",
		]);
	});

	test("delegated work is folded where it fell, saying what it checked and its steps, and opens to its steps", async () => {
		await draw();
		const fold = container.querySelector<HTMLButtonElement>(".call-work-head")!;
		expect(fold.textContent).toBe("Searched the voice log, Read the logins· 2 steps");
		expect(fold.getAttribute("aria-expanded")).toBe("false");
		expect(container.querySelector(".trail")).toBeNull();
		// It sits between the two lines it fell between.
		const rows = [...container.querySelectorAll(".call-transcript > li")].map((li) => (li.querySelector(".call-work") ? "work" : li.querySelector(".call-who")?.textContent));
		expect(rows).toEqual(["You", "Pork Chop", "work", "Pork Chop"]);
		await act(async () => fold.click());
		expect(fold.getAttribute("aria-expanded")).toBe("true");
		expect(container.querySelector(".trail")?.textContent).toContain("Searched the voice log");
		expect(container.querySelector(".trail")?.textContent).toContain("Read the logins");
	});

	test("while the teammate is still at work the fold says what it is doing now", async () => {
		const running: TranscriptEvent[] = [transcript[0]!, { kind: "tool", id: "t9", ts: 3, toolCallId: "tc9", title: "cargo test", status: "in_progress", said: { key: "run", one: "Ran the tests", many: "Ran the tests {n} times", doing: "Running the tests" } }];
		await draw({ events: running, working: true });
		expect(container.querySelector(".call-work-head")?.textContent).toContain("Running the tests");
		expect(container.querySelector(".call-work-dot")?.hasAttribute("data-live")).toBe(true);
	});

	test("the composer is there while live, as today, and gone once the call ends", async () => {
		const { powersOf } = await import("../src/dock");
		expect(powersOf("call", "live").say).toBe(true);
		expect(powersOf("call", "closed").say).toBe(false);
		await draw();
		expect(container.querySelector('[aria-label="Message"]')).not.toBeNull();
	});

	test("a call's transcript says each sentence once, with no written reply to open", async () => {
		const spoken = "Quite a bit, mostly QA.\n\nMidweek I audited the board.\n\nThen you ran backend switching.";
		await draw({
			events: [
				{ kind: "user", id: "u", ts: 1, text: "What did we talk about this week?" },
				{ kind: "agent", id: "r", ts: 2, text: "Quite a bit, mostly QA.", spoken },
				{ kind: "agent", id: "r-2", ts: 2, text: "Midweek I audited the board." },
				{ kind: "agent", id: "r-3", ts: 2, text: "Then you ran backend switching." },
			],
		});
		for (const line of ["Quite a bit, mostly QA.", "Midweek I audited the board.", "Then you ran backend switching."]) expect(container.textContent?.split(line)).toHaveLength(2);
		expect(container.textContent).not.toContain("Written reply");
	});

	test("a reply whose versions differ is said once, with its whole written version under it", async () => {
		await draw({
			events: [
				{ kind: "agent", id: "d", ts: 2, text: "The build fails for two reasons:", spoken: "Two things are wrong, both fixes are in the chat." },
				{ kind: "agent", id: "d-2", ts: 2, text: "1. **Missing API key.**\n2. **Port 8080 in use.**" },
			],
		});
		expect(text(".call-line .call-words > p")).toEqual(["Two things are wrong, both fixes are in the chat."]);
		const written = container.querySelector("details.call-written")!;
		expect(written.querySelector("summary")?.textContent).toBe("Written reply");
		expect(written.textContent).toContain("Missing API key.");
	});

	test("a card the call raised is a line home, saying how it stands", async () => {
		const card: TranscriptEvent = { kind: "human_action", id: "human:q1", ts: 1, actionId: "q1", reason: "Use blue?", question: true, status: "pending", from: { kind: "call", thread: "c1", title: "Release checks" } };
		await draw({ events: [card] });
		const home = [...container.querySelectorAll<HTMLButtonElement>("button")].find((one) => one.textContent?.includes("Open card in home chat"))!;
		expect(home.textContent).toContain("Needs you: Use blue?");
		await act(async () => home.click());
		expect(opened).toEqual([{ thread: { kind: "dm", key: "pork" }, eventId: "human:q1" }]);
		await draw({ events: [{ ...card, status: "done" }] });
		expect(container.textContent).toContain("Settled: Use blue?");
	});

	test("its own link line is not drawn as a line of the transcript", async () => {
		await draw({ events: [{ kind: "link", id: "link:call:c1", ts: 1, thread: "c1", threadKind: "call", title: "Release checks", state: "live" }, transcript[0]!] });
		expect(container.querySelectorAll(".call-transcript > li")).toHaveLength(1);
	});
});

test("a call's clock counts minutes and seconds, and hours past the hour", () => {
	expect(callClock(0)).toBe("0:00");
	expect(callClock(768_000)).toBe("12:48");
	expect(callClock(3_729_000)).toBe("1:02:09");
});

test("a call is hung up from the desk's Voice disconnected until the person speaks on it again", () => {
	const gone: TranscriptEvent = { kind: "notice", id: "n", ts: 9, level: "info", text: "Voice disconnected. This thread stays open." };
	expect(hangUpOf(transcript)).toBeUndefined();
	expect(hangUpOf([...transcript, gone])).toBe(9);
	expect(hangUpOf([...transcript, gone, { kind: "agent", id: "late", ts: 10, text: "The reply is ready." }])).toBe(9);
	expect(hangUpOf([...transcript, gone, { kind: "user", id: "again", ts: 11, text: "Back on." }])).toBeUndefined();
});
