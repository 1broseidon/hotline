import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { TranscriptEvent } from "../src/generated/contract";
import type { Block } from "../src/scheduledRuns";
import { installDom } from "./dom";

// A streamed reply draws only what changed, and never trips React's update
// guard (task-412). The whole window is under test, fed by a desk's socket:
// what drew every row again was a callback made in App, and what tripped the
// guard was an effect in the transcript setting state in the conversation
// around it, once for every publish of the thread's store.

const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });
const { App } = await import("../src/App");
const { replaceDesks } = await import("../src/desks");
const { bubbleId } = await import("../src/pacing");
const { groupSent } = await import("../src/scheduledRuns");
const { keepBlocks, toBlocks, withStreaming } = await import("../src/transcriptFolds");

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

/**
 * A teammate of this file's own: a thread's store outlives the test that
 * opened it, still watching through that test's socket.
 */
let ADA: string;
let made = 0;

/** Ada's row in the roster: at work or at rest, and when it last said something. */
function ada(state: "thinking" | "ready", latest: number) {
	return {
		persona: {
			id: ADA,
			name: "Ada",
			goal: "",
			backendId: "hotline",
			cwd: "/work",
			mcpPolicy: { mode: "all", serverIds: [] },
			skillPolicy: { mode: "all", names: [] },
			backgroundWork: false,
			allowedSenders: [],
			sessionCheckpoints: [],
			createdAt: 0,
			updatedAt: 0,
		},
		session: {
			personaId: ADA,
			state,
			contextRestored: false,
			models: [],
			modes: [],
			configs: [],
			slashCommands: [],
			capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
		},
		waiting: false,
		drawing: false,
		latest,
	};
}

/** What Ada's conversation holds when the window opens it: said long ago, so none of it waits for the beat bubbles land to. */
let said: TranscriptEvent[];

/** The desk's end of the socket: answers what the window asks, and says what the test has it say. */
class FakeDesk {
	static readonly OPEN = 1;
	static socket: FakeDesk | null = null;
	readyState = 1;
	onopen: (() => void) | null = null;
	onmessage: ((message: { data: string }) => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	/** The subscriptions the window made, by what they watch. */
	roster = 0;
	thread = 0;
	constructor(readonly url: string) {
		FakeDesk.socket = this;
		setTimeout(() => this.onopen?.(), 0);
	}
	send(data: string) {
		const frame = JSON.parse(data) as Record<string, unknown>;
		const id = frame["id"] as number;
		const reply = (answer: Record<string, unknown>) => setTimeout(() => this.onmessage?.({ data: JSON.stringify(answer) }), 0);
		if ("sub" in frame) {
			const target = frame["sub"] as { view?: string; threadId?: { kind: string; key: string } };
			reply({ id, ok: true });
			if (target.view === "roster") {
				this.roster = id;
				reply({ sub: id, snapshot: [ada("thinking", 1)] });
			} else if (target.threadId?.kind === "dm" && target.threadId.key === ADA) {
				this.thread = id;
				reply({ sub: id, snapshot: said });
			} else reply({ sub: id, snapshot: [] });
			return;
		}
		if (frame["cmd"] === "client.hello") reply({ id, ok: true, result: { capabilities: ["threads", "threads2"] } });
		else reply({ id, ok: false, error: "Not in this test." });
	}
	/** One frame from the desk, as the socket hands it over. */
	say(frame: Record<string, unknown>) {
		this.onmessage?.({ data: JSON.stringify(frame) });
	}
	/** Words of a reply still being written. */
	words(messageId: string, text: string) {
		this.say({ sub: this.thread, ephemeral: { type: "thread_delta", thread: { kind: "dm", key: ADA }, messageId, text, kind: "text" } });
	}
	close() {
		if (this.readyState === 3) return;
		this.readyState = 3;
		this.onclose?.();
	}
}

let act: typeof import("react").act;
let root: Root | null;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let frames: FrameRequestCallback[];

beforeEach(async () => {
	frames = [];
	FakeDesk.socket = null;
	ADA = `ada-streaming-${++made}`;
	said = [{ kind: "user", id: "asked", ts: 1, text: "Write it all down" }];
	({ teardown } = installDom({
		ResizeObserver: NoResize,
		WebSocket: FakeDesk,
		requestAnimationFrame: (callback: FrameRequestCallback) => frames.push(callback),
		cancelAnimationFrame: () => {},
	}));
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "ws://core", token: "t" }]);
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => {
		root?.unmount();
		root = null;
	});
	replaceDesks([]);
	await teardown();
});

/** The frames asked for, run. */
function frame() {
	const due = frames;
	frames = [];
	for (const one of due) one(performance.now());
}

/** Timers and frames, until the window has taken in what it was sent. */
async function settle() {
	for (let turn = 0; turn < 8; turn++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 5));
			frame();
		});
	}
}

/** The window, with Ada's conversation open in it. */
async function open(): Promise<FakeDesk> {
	await act(async () => {
		root!.render(<App />);
	});
	await settle();
	await act(async () => {
		container.querySelector<HTMLButtonElement>('[data-teammate-row][aria-label^="Ada"]')!.click();
	});
	await settle();
	const desk = FakeDesk.socket!;
	if (desk.thread === 0) throw new Error(`The conversation was not opened: ${container.textContent}`);
	return desk;
}

const paragraph = (n: number) => `Paragraph ${n} of the reply, long enough to stand as a bubble of its own in the conversation.`;

describe("a long reply never trips React's update guard", () => {
	// Every frame from the desk is a task of its own, and a burst of them
	// arrives faster than it is drawn: the socket's listener runs again and
	// again with nothing between two runs but the render the last one asked
	// for at once. React's scheduler gets no turn, which here is everything
	// happening inside one `act`.

	test("written down as ninety-six lines in one burst, it is drawn, and nothing is thrown out of the socket's listener", async () => {
		const desk = await open();
		let thrown: unknown = null;
		await act(async () => {
			try {
				for (let n = 0; n < 96; n++) {
					desk.say({ sub: desk.thread, event: { kind: "agent", id: bubbleId("reply", n), ts: Date.now(), text: paragraph(n) } });
					// The roster says what the teammate is doing between every few.
					if (n % 4 === 3) desk.say({ sub: desk.roster, event: ada("thinking", 2 + n) });
					await Promise.resolve();
				}
				desk.say({ sub: desk.thread, event: { kind: "turn", id: "end", ts: Date.now(), stopReason: "end_turn" } });
				desk.say({ sub: desk.roster, event: ada("ready", 99) });
			} catch (error) {
				thrown = error;
			}
		});
		expect(thrown).toBeNull();
		await settle();
		// Its bubbles land to a beat, the first of them by now.
		expect(container.querySelector('[data-event-id="reply"]')?.textContent).toContain(paragraph(0));
	});

	test("streaming in as ninety-six bubbles, a frame apart, it throws nothing either: no publish asks for a second render from inside the first", async () => {
		const desk = await open();
		let thrown: unknown = null;
		await act(async () => {
			try {
				for (let n = 0; n < 96; n++) {
					desk.words("reply", `${paragraph(n)}\n\n`);
					// The frame the words are folded on, and the render it asks for.
					frame();
					await Promise.resolve();
				}
			} catch (error) {
				thrown = error;
			}
		});
		expect(thrown).toBeNull();
		await settle();
		expect(container.querySelector('[data-event-id="reply"]')?.textContent).toContain(paragraph(0));
	});
});

describe("a streamed reply draws only what changed", () => {
	/**
	 * How often each line's row has been drawn. A row reads its line's
	 * `reactions` once each time it is drawn, and nothing else reads them, so
	 * every line the socket hands the window is given a `reactions` that counts.
	 */
	let drawn: Map<string, number>;

	beforeEach(() => {
		drawn = new Map();
		const parse = JSON.parse;
		spyOn(JSON, "parse").mockImplementation((text: string) => {
			const frame = parse(text) as { snapshot?: unknown; event?: unknown };
			for (const line of [...(Array.isArray(frame.snapshot) ? frame.snapshot : []), frame.event]) {
				if (typeof line !== "object" || line === null || !("kind" in line) || !("id" in line)) continue;
				const id = String(line.id);
				Object.defineProperty(line, "reactions", {
					get() {
						drawn.set(id, (drawn.get(id) ?? 0) + 1);
						return undefined;
					},
				});
			}
			return frame;
		});
	});

	/** The rows drawn since `before`, by line, with how many times. */
	const since = (before: Map<string, number>) => Object.fromEntries([...drawn].flatMap(([id, count]) => (count === (before.get(id) ?? 0) ? [] : [[id, count - (before.get(id) ?? 0)]])));

	test("rows other than the live one are not drawn again for a roster update or for a bubble that streams in", async () => {
		// Two turns of talk, and the line the reply under way answers.
		said = [
			{ kind: "user", id: "u1", ts: 1, text: "Plan the release" },
			{ kind: "agent", id: "a1", ts: 2, text: "Here is the plan." },
			{ kind: "agent", id: "a2", ts: 3, text: "It has three steps." },
			{ kind: "turn", id: "t1", ts: 4, stopReason: "end_turn" },
			{ kind: "user", id: "u2", ts: 5, text: "Start on the first" },
			{ kind: "agent", id: "a3", ts: 6, text: "The first step is done." },
			{ kind: "turn", id: "t2", ts: 7, stopReason: "end_turn" },
			{ kind: "user", id: "u3", ts: 8, text: "Now write it all down" },
		];
		const desk = await open();
		const bubbles = ["u1", "a1", "a2", "u2", "a3", "u3"];
		for (const id of bubbles) expect(container.querySelector(`[data-event-id="${id}"]`)).not.toBeNull();
		// The count counts: every bubble in the page has been drawn.
		for (const id of bubbles) expect(drawn.get(id)).toBeGreaterThan(0);

		// The roster says something new about the teammate, as it does every few lines of a reply.
		let before = new Map(drawn);
		for (let latest = 9; latest < 12; latest++) {
			await act(async () => desk.say({ sub: desk.roster, event: ada("thinking", latest) }));
			await settle();
		}
		expect(since(before)).toEqual({});
		// Nor when the roster says the turn has ended, and that another has begun.
		for (const state of ["ready", "thinking"] as const) {
			await act(async () => desk.say({ sub: desk.roster, event: ada(state, 12) }));
			await settle();
		}
		expect(since(before)).toEqual({});

		// The reply starts to stream: its first bubble is whole and lands.
		before = new Map(drawn);
		await act(async () => {
			desk.words("reply", `${paragraph(0)}\n\n`);
			frame();
		});
		await settle();
		expect(container.querySelector('[data-event-id="reply"]')?.textContent).toContain(paragraph(0));
		expect(since(before)).toEqual({});

		// More of it streams in, a bubble at a time, with the roster's word between.
		for (let n = 1; n < 6; n++) {
			await act(async () => {
				desk.words("reply", `${paragraph(n)}\n\n`);
				frame();
			});
			await act(async () => desk.say({ sub: desk.roster, event: ada("thinking", 20 + n) }));
			await settle();
		}
		expect(since(before)).toEqual({});
	});

	const line = (id: string, ts: number, text: string): TranscriptEvent => ({ kind: "agent", id, ts, text });
	const live = (text: string) => [{ messageId: "reply", kind: "agent" as const, ts: 9, text }];
	const idsOf = (blocks: Block[]) => blocks.map((block) => (block.kind === "event" ? block.event.id : block.id));

	test("of a reply streaming in, only the newest bubble is a new block: the bubbles before it are the blocks they were", () => {
		const written = toBlocks([{ kind: "user", id: "u1", ts: 1, text: "Go" }]);
		const first = keepBlocks([], withStreaming(written, live(`${paragraph(0)}\n\n${paragraph(1)}\n\n`)));
		const second = keepBlocks(first, withStreaming(written, live(`${paragraph(0)}\n\n${paragraph(1)}\n\n${paragraph(2)}\n\n`)));
		expect(idsOf(second)).toEqual(["u1", "reply", "reply-2", "reply-3"]);
		expect(second[0]).toBe(first[0]!);
		expect(second[1]).toBe(first[1]!);
		expect(second[2]).toBe(first[2]!);
		// A bubble that says something else is a new block, and so is one whose line has been written.
		const grown = keepBlocks(second, withStreaming(written, live(`${paragraph(0)}\n\nIt changed.\n\n${paragraph(2)}\n\n`)));
		expect(grown[1]).toBe(second[1]!);
		expect(grown[2]).not.toBe(second[2]!);
		const landed = keepBlocks(second, toBlocks([{ kind: "user", id: "u1", ts: 1, text: "Go" }, line("reply", 9, paragraph(0))]));
		expect(landed[1]).not.toBe(second[1]!);
	});

	test("a line landing leaves every run of steps and of files that it is not part of the block it was", () => {
		const file = (id: string, ts: number): TranscriptEvent => ({ kind: "agent", id, ts, text: "", attachments: [{ kind: "image", name: `${id}.png`, mimeType: "image/png", size: 1, path: `/tmp/${id}.png` }] });
		const events: TranscriptEvent[] = [
			{ kind: "user", id: "u1", ts: 1, text: "Go" },
			{ kind: "tool", id: "s1", ts: 2, toolCallId: "s1", title: "Read", status: "completed" },
			{ kind: "tool", id: "s2", ts: 3, toolCallId: "s2", title: "Edit", status: "completed" },
			line("a1", 4, "Done."),
			{ kind: "turn", id: "t1", ts: 5, stopReason: "end_turn" },
			file("f1", 6),
			file("f2", 7),
		];
		const first = keepBlocks([], groupSent(toBlocks(events)));
		const second = keepBlocks(first, groupSent(toBlocks([...events, { kind: "user", id: "u2", ts: 8, text: "Thanks" }])));
		const steps = first.find((block) => block.kind === "steps")!;
		const sent = first.find((block) => block.kind === "sent")!;
		expect(second).toContain(steps);
		expect(second).toContain(sent);
		// A step that changed makes its run a new block.
		const again = keepBlocks(second, groupSent(toBlocks(events.map((event) => (event.id === "s2" ? { ...event, title: "Edit again" } : event)))));
		expect(again).not.toContain(steps);
		expect(again).toContain(sent);
	});
});
