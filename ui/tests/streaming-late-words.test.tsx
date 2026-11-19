import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { StreamDelta, ThreadId, TranscriptEvent } from "../src/generated/contract";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

/*
 * The desk sends a thread's words and its written lines on two channels, and
 * a socket's subscription takes whichever is ready first (wire/subscribe.rs),
 * so the last words of a reply can reach the window after the line that
 * carries them, and after the turn's end. Words for a line already written
 * are stale: drawn, they put the reply on screen a second time, under the
 * same ids as the written bubbles, at the bottom of the conversation.
 */

let act: typeof import("react").act;
let useSyncExternalStore: typeof import("react").useSyncExternalStore;
let Transcript: typeof import("../src/components/Transcript").Transcript;
let ThreadStore: typeof import("../src/tape").ThreadStore;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let frames: Map<number, FrameRequestCallback>;
let timers: Map<number, () => void>;
type Handlers = { snapshot(items: TranscriptEvent[]): void; event(item: TranscriptEvent): void; ephemeral(delta: StreamDelta): void };
/** Each thread's subscription, by its key: the main conversation and a work thread open beside it. */
let subscriptions: Map<string, Handlers>;
const dm: ThreadId = { kind: "dm", key: "late-words" };
const side: ThreadId = { kind: "side", key: "late-words-side" };
let store: InstanceType<typeof ThreadStore>;
let wire: Wire;
let consoleError: typeof console.error;
let errors: string[];

/** Old enough that the beat bubbles land to lets them all land at once. */
const WRITTEN = 1_000;

const PARAGRAPHS = [
	"The fix is in the store: a word for a line already written is dropped as it arrives.",
	"Each paragraph of this reply is long enough to stand as a bubble of its own on screen.",
	"And the third closes the reply, with enough words that it never joins the one above it.",
];
const REPLY = PARAGRAPHS.join("\n\n");

beforeEach(async () => {
	frames = new Map();
	timers = new Map();
	subscriptions = new Map();
	let frameId = 0;
	let timerId = 0;
	const installed = installDom({
		requestAnimationFrame: (callback: FrameRequestCallback) => { frames.set(++frameId, callback); return frameId; },
		cancelAnimationFrame: (id: number) => frames.delete(id),
		setTimeout: (callback: () => void) => { timers.set(++timerId, callback); return timerId; },
		clearTimeout: (id: number) => timers.delete(id),
	});
	teardown = installed.teardown;
	Object.assign(globalThis, { ResizeObserver: installed.dom.ResizeObserver, MutationObserver: installed.dom.MutationObserver, CSS: installed.dom.CSS });
	({ act, useSyncExternalStore } = await import("react"));
	({ ThreadStore } = await import("../src/tape"));
	({ Transcript } = await import("../src/components/Transcript"));
	wire = {
		onConnection(callback: (state: string) => void) { callback("open"); return () => {}; },
		subscribe(target: { threadId: ThreadId }, next: Handlers) { subscriptions.set(target.threadId.key, next); return () => {}; },
	} as unknown as Wire;
	store = new ThreadStore(wire, dm);
	store.acquire();
	on(dm).snapshot([{ kind: "user", id: "ask", ts: WRITTEN, text: "Find out why the reply drew twice" }]);
	container = document.createElement("div");
	root = (await import("react-dom/client")).createRoot(container);
	// React says so when two rows share a key, which is what a stale copy is.
	errors = [];
	consoleError = console.error;
	console.error = (...args: unknown[]) => { errors.push(args.map(String).join(" ")); };
});
afterEach(async () => {
	console.error = consoleError;
	await act(async () => root.unmount());
	await teardown();
});

function on(thread: ThreadId): Handlers {
	return subscriptions.get(thread.key)!;
}
function words(messageId: string, text: string, thread: ThreadId = dm) {
	on(thread).ephemeral({ type: "thread_delta", thread, messageId, text, kind: "text" });
}
/** A frame and the burst's wait go by: everything queued is folded. */
function settleDown() {
	for (let pass = 0; pass < 3; pass++) {
		const due = [...frames.values()];
		frames.clear();
		for (const callback of due) callback(0);
		const waited = [...timers.values()];
		timers.clear();
		for (const callback of waited) callback();
	}
}
/** The reply written down, a line a bubble, as the desk writes it. */
function written(base = "reply", paragraphs = PARAGRAPHS, ts = WRITTEN + 10): TranscriptEvent[] {
	return paragraphs.map((text, index) => ({ kind: "agent", id: index === 0 ? base : `${base}-${index + 1}`, ts, text }));
}
const turnEnd: TranscriptEvent = { kind: "turn", id: "end", ts: WRITTEN + 11, stopReason: "end_turn" };

/**
 * The turn as it happened: an acknowledgement, a tool call, two lines the
 * person sent while it worked, then the reply streaming as words.
 */
function turnUpToTheReply(thread: ThreadId = dm) {
	const handlers = on(thread);
	handlers.event({ kind: "agent", id: "ack", ts: WRITTEN + 1, text: "On it." });
	const tool = { kind: "tool" as const, id: "tool", toolCallId: "call", ts: WRITTEN + 2, title: "Read tape.ts", status: "pending" as const };
	handlers.event(tool);
	handlers.event({ ...tool, status: "completed" });
	handlers.event({ kind: "user", id: "steer-1", ts: WRITTEN + 3, text: "also check the dock" });
	handlers.event({ kind: "user", id: "steer-2", ts: WRITTEN + 4, text: "and the phone" });
	settleDown();
}

async function render() {
	function Reader() {
		const state = useSyncExternalStore(store.subscribe, store.snapshot);
		return <Transcript personaId="late-words" name="Test" events={state.events} streaming={state.streaming} live focus={null} />;
	}
	await act(async () => root.render(<Reader />));
}

/** The rows on screen, by the id each is drawn under, top to bottom. */
function rows(): string[] {
	return [...container.querySelectorAll("[data-event-id]")].map((row) => row.getAttribute("data-event-id")!);
}

function expectReplyOnceUnderTheSteers() {
	const drawn = rows();
	for (const id of ["reply", "reply-2", "reply-3"]) expect(drawn.filter((one) => one === id)).toHaveLength(1);
	const text = container.textContent ?? "";
	for (const paragraph of PARAGRAPHS) expect(text.split(paragraph)).toHaveLength(2);
	// The reply sits where it was written: after the lines the person sent while the turn worked.
	expect(drawn.indexOf("reply")).toBeGreaterThan(drawn.indexOf("steer-2"));
	expect(drawn.indexOf("reply-3")).toBe(drawn.indexOf("reply-2") + 1);
	expect(errors.filter((error) => error.includes("same key"))).toEqual([]);
}

for (const late of ["before the turn's end", "after the turn's end"] as const) {
	test(`the reply's last words arriving after its lines, ${late}, draw nothing more`, async () => {
		await render();
		await act(async () => {
			turnUpToTheReply();
			words("reply", `${PARAGRAPHS[0]}\n\n`);
			settleDown();
			for (const line of written()) on(dm).event(line);
			if (late === "after the turn's end") on(dm).event(turnEnd);
			// The words sent before those lines, heard after them.
			words("reply", `${PARAGRAPHS[1]}\n\n${PARAGRAPHS[2]}\n\n`);
			settleDown();
		});
		expectReplyOnceUnderTheSteers();
		await act(async () => {
			if (late === "before the turn's end") on(dm).event(turnEnd);
			settleDown();
		});
		expectReplyOnceUnderTheSteers();
		expect(store.snapshot().streaming).toEqual([]);
	});
}

test("a reply whose words all arrive after its lines is drawn once", async () => {
	await render();
	await act(async () => {
		turnUpToTheReply();
		for (const line of written()) on(dm).event(line);
		on(dm).event(turnEnd);
		words("reply", `${REPLY}\n\n`);
		settleDown();
	});
	expectReplyOnceUnderTheSteers();
	expect(store.snapshot().streaming).toEqual([]);
});

test("words heard after a reconnect's snapshot already holds their line are not drawn again", async () => {
	await render();
	await act(async () => {
		turnUpToTheReply();
		words("reply", `${PARAGRAPHS[0]}\n\n`);
		settleDown();
		// The socket dropped and came back: the snapshot holds the reply, written while it was down.
		on(dm).snapshot([...store.snapshot().events, ...written()]);
		words("reply", `${PARAGRAPHS[1]}\n\n${PARAGRAPHS[2]}\n\n`);
		settleDown();
	});
	expectReplyOnceUnderTheSteers();
	expect(store.snapshot().streaming).toEqual([]);
	await act(async () => {
		on(dm).event(turnEnd);
		settleDown();
	});
	expectReplyOnceUnderTheSteers();
});

test("two streamed messages in one turn: words late for the first do not draw it again under the second", async () => {
	await render();
	const second = ["A second message after another tool call, long enough to be a bubble on its own."];
	await act(async () => {
		turnUpToTheReply();
		words("reply", `${PARAGRAPHS[0]}\n\n`);
		settleDown();
		for (const line of written()) on(dm).event(line);
		settleDown();
		on(dm).event({ kind: "tool", id: "tool-2", toolCallId: "call-2", ts: WRITTEN + 20, title: "Read wire.ts", status: "completed" });
		words("later", `${second[0]}\n\n`);
		// The first message's last words, heard while the second streams.
		words("reply", `${PARAGRAPHS[1]}\n\n${PARAGRAPHS[2]}\n\n`);
		settleDown();
	});
	expectReplyOnceUnderTheSteers();
	await act(async () => {
		for (const line of written("later", second, WRITTEN + 21)) on(dm).event(line);
		on(dm).event({ ...turnEnd, ts: WRITTEN + 22 });
		settleDown();
	});
	expectReplyOnceUnderTheSteers();
	expect(rows().filter((id) => id === "later")).toHaveLength(1);
	expect(store.snapshot().streaming).toEqual([]);
});

test("the main conversation's late words stay its own while a work thread streams beside it", async () => {
	const work = new ThreadStore(wire, side);
	work.acquire();
	on(side).snapshot([{ kind: "user", id: "brief", ts: WRITTEN, text: "Look at the dock" }]);
	await render();
	await act(async () => {
		turnUpToTheReply();
		words("reply", `${REPLY}\n\n`, side);
		words("reply", `${PARAGRAPHS[0]}\n\n`);
		settleDown();
		for (const line of written()) on(dm).event(line);
		on(dm).event(turnEnd);
		words("reply", `${PARAGRAPHS[1]}\n\n${PARAGRAPHS[2]}\n\n`);
		settleDown();
	});
	expectReplyOnceUnderTheSteers();
	expect(store.snapshot().streaming).toEqual([]);
	// The work thread's words are its own, still waiting for their line there.
	expect(work.snapshot().streaming.map((one) => one.messageId)).toEqual(["reply"]);
	work.release();
});
