import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";
import { layOut } from "./transcript-layout";

// Mermaid off the scroll path (task-413). Its script is megabytes and a
// drawing is a frame or more of work, so neither is done as a diagram
// scrolls into view: a thread's diagrams are drawn ahead of the view while
// the window is at rest, a row is measured with its drawing in it, and a
// thread with no diagram never asks for the script.
//
// Time here is the test's: the window's timers run when `after` says so and
// the clock reads what it says, so "at rest for 300 ms" is exact however
// loaded the machine is. Mermaid is a stand-in the test hands to the script
// tag (tests/preload.ts): it draws a box of the size its source names.

const ROW = 100;
const VIEW = 500;
/** A diagram's row while its source shows, and once its drawing does: what the test's layout says they are. */
const AS_SOURCE = 400;
const AS_DRAWING = 220;
/** The same for a test whose diagram is taller drawn than as its source: null for the two above. */
let grows: { source: number; drawing: number } | null;

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let Transcript: typeof import("../src/components/Transcript").Transcript;
let frames: FrameRequestCallback[];
let observed: { target: Element; callback: IntersectionObserverCallback }[];
let watchers: Watcher[];
let tall: Map<string, number>;
let restore: () => void;
let timers: { id: number; at: number; run: () => void }[];
let now: number;
/** What the stand-in mermaid was asked to draw, in order. */
let rendered: string[];
/** A teammate, and a mark in every diagram's source, of this test's own: heights are remembered by thread and drawings by source, across tests. */
let ada: string;
let made = 0;

/** A stand-in IntersectionObserver: `nearby` says what has come near the view. */
class Observer {
	constructor(private callback: IntersectionObserverCallback) {}
	observe(target: Element) {
		observed.push({ target, callback: this.callback });
	}
	disconnect() {
		observed = observed.filter((one) => one.callback !== this.callback);
	}
	unobserve() {}
	takeRecords() {
		return [];
	}
}

/** A stand-in ResizeObserver the test fires by hand, as the engine does when a row's box changes. */
class Watcher {
	targets = new Set<Element>();
	constructor(private callback: ResizeObserverCallback) {
		watchers.push(this);
	}
	observe(target: Element) {
		this.targets.add(target);
	}
	unobserve(target: Element) {
		this.targets.delete(target);
	}
	disconnect() {
		this.targets.clear();
	}
	fire(...targets: Element[]) {
		const entries = targets.filter((target) => this.targets.has(target)).map((target) => ({ target }) as ResizeObserverEntry);
		if (entries.length > 0) this.callback(entries, this as unknown as ResizeObserver);
	}
}

/** Mermaid, as far as the window uses it: a source that says `%% WxH` is drawn that size, and one that says `broken` does not parse. */
const mermaid = {
	initialize() {},
	async parse(source: string) {
		if (source.includes("broken")) throw new Error("Parse error on line 2");
	},
	async render(_id: string, source: string) {
		rendered.push(source);
		const [, width, height] = /%% (\d+)x(\d+)/.exec(source) ?? [];
		return { svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="100%" style="max-width: ${width}px;"><g/></svg>` };
	},
};

beforeEach(async () => {
	frames = [];
	observed = [];
	watchers = [];
	timers = [];
	rendered = [];
	grows = null;
	now = 1_000;
	let timerId = 0;
	const installed = installDom({
		requestAnimationFrame: (callback: FrameRequestCallback) => frames.push(callback),
		cancelAnimationFrame: () => {},
		IntersectionObserver: Observer,
		ResizeObserver: Watcher,
		// Read bare, as a page reads it, where the window's colours are taken for a drawing.
		getComputedStyle: (element: Element) => window.getComputedStyle(element),
	});
	teardown = installed.teardown;
	// An engine that does not anchor the scroll (WebKitGTK): what holds the view is the transcript's own.
	Object.assign(globalThis, {
		MutationObserver: installed.dom.MutationObserver,
		CSS: { escape: installed.dom.CSS.escape, supports: () => false },
		DOMParser: installed.dom.DOMParser,
		XMLSerializer: installed.dom.XMLSerializer,
	});
	// The window's timers and its clock, under the test's hand.
	Object.assign(installed.dom, {
		setTimeout: (run: () => void, ms = 0) => {
			timers.push({ id: ++timerId, at: now + ms, run });
			return timerId;
		},
		clearTimeout: (id: number) => {
			timers = timers.filter((timer) => timer.id !== id);
		},
	});
	spyOn(performance, "now").mockImplementation(() => now);
	({ tall, restore } = layOut({ row: ROW, view: VIEW }));
	// A row holding a diagram is as tall as what it shows: its source, or its drawing (or the box kept for one).
	const plain = tall.get.bind(tall);
	tall.get = (id: string) => {
		const row = container.querySelector(`[data-event-id="${id}"], [data-probe="${id}"]`);
		if (row?.querySelector(".mermaid-diagram")) return grows?.drawing ?? AS_DRAWING;
		if (row?.querySelector(".diagram-source")) return grows?.source ?? AS_SOURCE;
		return plain(id);
	};
	ada = `ada-diagrams-${++made}`;
	act = (await import("react")).act;
	({ Transcript } = await import("../src/components/Transcript"));
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	Reflect.deleteProperty(globalThis, "mermaid");
	restore();
	await teardown();
});

function lines(from: number, to: number): TranscriptEvent[] {
	const said: TranscriptEvent[] = [];
	for (let n = from; n <= to; n++) said.push({ kind: n % 2 ? "agent" : "user", id: `e${n}`, ts: n, text: `Line ${n}` } as TranscriptEvent);
	return said;
}

/** An agent's line holding one diagram, `size` across and down once drawn. The mark makes its source this test's alone. */
function diagram(id: string, ts: number, size = "300x120", body = "graph TD\n  A-->B\n  B-->C\n  A-->C"): TranscriptEvent {
	return { kind: "agent", id, ts, text: `Here it is:\n\n\`\`\`mermaid\n%% ${size} ${ada} ${id}\n${body}\n\`\`\`` } as TranscriptEvent;
}

/** The thread with one line swapped for another. */
function withLine(events: TranscriptEvent[], line: TranscriptEvent): TranscriptEvent[] {
	return events.map((event) => (event.ts === line.ts ? line : event));
}

function draw(events: TranscriptEvent[], personaId = ada) {
	root.render(<Transcript personaId={personaId} name="Ada" events={events} streaming={[]} live={false} focus={null} />);
}

const scroller = () => container.querySelector(".scroller") as unknown as HTMLElement;
const rowOf = (id: string) => container.querySelector(`.tape-column > [data-event-id="${id}"]`) as unknown as HTMLElement | null;
const probeOf = (id: string) => container.querySelector(`.tape-probe > [data-probe="${id}"]`) as unknown as HTMLElement | null;
const topOf = (id: string) => rowOf(id)?.getBoundingClientRect().top;
const scriptTag = () => document.head.querySelector("script[data-mermaid]") as unknown as HTMLScriptElement | null;
/** What stands in a diagram's place in a row: its source, an empty box of the drawing's size, or the picture. */
function standing(row: HTMLElement | null): { what: "source" | "box" | "picture" | "nothing"; style: string | null; unsized: boolean } {
	const body = row?.querySelector(".diagram-body");
	const drawing = body?.querySelector(".mermaid-diagram");
	const what = drawing ? (drawing.tagName === "IMG" ? "picture" : "box") : body?.querySelector(".diagram-source") ? "source" : "nothing";
	return { what, style: drawing?.getAttribute("style") ?? null, unsized: body?.hasAttribute("data-unsized") ?? false };
}

/** The frames asked for, run. */
async function frame() {
	await act(async () => {
		const due = frames;
		frames = [];
		for (const one of due) one(0);
	});
}

/** Turns of the real event loop, for what no timer of the window's carries: a module's import, a promise's chain. */
async function turns() {
	for (let turn = 0; turn < 4; turn++) await act(async () => new Promise((done) => setTimeout(done, 0)));
}

/**
 * Lets `ms` of the window's time pass: every timer due in it runs at its own
 * moment, in order, and the frames each asks for are drawn. With `until`,
 * it stops the moment that holds, before another frame is drawn: what stands
 * then is what the next frame would show.
 */
async function after(ms: number, until?: () => boolean) {
	const end = now + ms;
	for (;;) {
		const due = timers.filter((timer) => timer.at <= end).sort((a, b) => a.at - b.at || a.id - b.id)[0];
		if (due === undefined) break;
		timers = timers.filter((timer) => timer !== due);
		now = Math.max(now, due.at);
		await act(async () => due.run());
		await turns();
		if (until?.()) return;
		for (let pass = 0; pass < 6 && frames.length > 0; pass++) await frame();
	}
	now = end;
}

/** A scroll of the person's own, and the frame the transcript answers it on. */
async function scrollTo(top: number) {
	scroller().scrollTop = top;
	await act(async () => {
		scroller().dispatchEvent(new window.Event("scroll") as unknown as Event);
	});
	await frame();
}

/** The thread opened, following its last line, and every row that can be measured off to the side measured. */
async function open(events: TranscriptEvent[]) {
	await act(async () => draw(events));
	await scrollTo(scroller().scrollHeight);
	// (A row waiting off to the side for its diagram keeps the measuring going: sixty frames is every other row.)
	for (let pass = 0; pass < 60 && frames.length > 0; pass++) await frame();
	await act(async () => nearby());
}

/** The blocks in the page are near the view; the ones measured off to the side never are. */
function nearby() {
	for (const { target, callback } of [...observed]) {
		if (target.closest(".tape-probe") === null) callback([{ isIntersecting: true, target } as unknown as IntersectionObserverEntry], {} as IntersectionObserver);
	}
}

/** Mermaid's script arrives: the tag the window put in the page loads, with the stand-in as what it defines. */
async function arrive() {
	const tag = scriptTag();
	if (tag === null) throw new Error("the window never asked for mermaid's script");
	Object.assign(globalThis, { mermaid });
	await act(async () => {
		tag.dispatchEvent(new window.Event("load") as unknown as Event);
	});
	await turns();
}

/** Long enough for a block's source to settle and the window to count as at rest, and for one diagram's turn. */
const A_TURN = 700;

describe("mermaid's script", () => {
	test("is never fetched by a thread with no diagram in it: prose, code, and a straight-line flow drawn as chips", async () => {
		const code: TranscriptEvent = { kind: "agent", id: "e21", ts: 21, text: "```ts\nconst mermaid = 1;\n```" } as TranscriptEvent;
		const chain: TranscriptEvent = { kind: "agent", id: "e41", ts: 41, text: "```mermaid\nflowchart LR\n  A[App] --> B[Embed query] --> C[Qdrant]\n```" } as TranscriptEvent;
		await open(withLine(withLine(lines(1, 60), code), chain));
		// Up the whole thread and back, with the window at rest between: every row is drawn at some point.
		await after(5 * A_TURN);
		await scrollTo(0);
		await after(5 * A_TURN);
		await scrollTo(scroller().scrollHeight);
		await after(5 * A_TURN);
		expect(container.querySelector(".diagram-body")).toBeNull();
		expect(document.head.querySelector("script")).toBeNull();

		// The same thread once a diagram is said in it does ask, so the tag is what a fetch looks like here.
		await act(async () => draw([...withLine(withLine(lines(1, 60), code), chain), diagram("d61", 61)]));
		await act(async () => nearby());
		await after(A_TURN);
		expect(scriptTag()?.getAttribute("src")).toBe("/assets/mermaid.min.js");
	});

	test("is fetched for a diagram far up a loaded thread, before anyone scrolls to it, and only once the window is at rest", async () => {
		await open(withLine(lines(1, 60), diagram("d5", 5)));
		// Far above the view: not in the page, only measured off to the side.
		expect(rowOf("d5")).toBeNull();
		expect(probeOf("d5")).not.toBeNull();

		// Scrolling, a scroll every 100 ms for three seconds: nothing is fetched under it.
		const foot = scroller().scrollTop;
		for (let step = 1; step <= 30; step++) {
			await scrollTo(foot - (step % 2) * 40);
			await after(100);
			expect(scriptTag()).toBeNull();
		}
		// A key every 100 ms does the same: typing is not rest.
		for (let step = 0; step < 10; step++) {
			document.dispatchEvent(new window.Event("keydown") as unknown as Event);
			await after(100);
		}
		expect(scriptTag()).toBeNull();

		// At rest, it is.
		await after(A_TURN);
		expect(scriptTag()).not.toBeNull();
		// One tag, however many turns pass before it arrives.
		await after(3 * A_TURN);
		expect(document.head.querySelectorAll("script").length).toBe(1);
	});

	test("is not fetched by a window that was only held up: what the person did meanwhile reaches it first, and the wait starts again", async () => {
		await open(withLine(lines(1, 60), diagram("d5", 5)));
		// Scrolling, long enough that the diagram has asked for its drawing and the asking waits for rest.
		const foot = scroller().scrollTop;
		for (let step = 1; step <= 5; step++) {
			await scrollTo(foot - (step % 2) * 40);
			await after(100);
		}
		expect(scriptTag()).toBeNull();
		// Half a second in which the window does nothing at all, not even its
		// timers: a long frame. The person scrolled in it, and the scroll is
		// still on its way when the window comes back and its timers run.
		now += 500;
		const overdue = timers.filter((timer) => timer.at <= now);
		timers = timers.filter((timer) => timer.at > now);
		await act(async () => {
			for (const timer of overdue) timer.run();
		});
		await turns();
		expect(scriptTag()).toBeNull();
		await scrollTo(foot - 80);
		await after(250);
		expect(scriptTag()).toBeNull();
		// Left alone for the whole of the wait, it is fetched.
		await after(A_TURN);
		expect(scriptTag()).not.toBeNull();
	});

	test("that fails to load leaves the source with a line saying so, and the next diagram to ask tries again", async () => {
		await open([...lines(1, 8), diagram("d9", 9)]);
		await after(A_TURN);
		const first = scriptTag()!;
		await act(async () => {
			first.dispatchEvent(new window.Event("error") as unknown as Event);
		});
		await turns();
		expect(standing(rowOf("d9"))).toMatchObject({ what: "source", unsized: false });
		expect(rowOf("d9")?.querySelector(".mermaid-note")?.textContent).toBe("Couldn't draw this diagram, so here is its source.");
		expect(scriptTag()).toBeNull();

		await act(async () => draw([...lines(1, 8), diagram("d9", 9), diagram("d10", 10)]));
		await act(async () => nearby());
		await after(A_TURN);
		expect(scriptTag()).not.toBeNull();
		await arrive();
		await after(4 * A_TURN);
		expect(standing(rowOf("d9")).what).toBe("picture");
		expect(standing(rowOf("d10")).what).toBe("picture");
		expect(container.querySelector(".mermaid-note")).toBeNull();
	});
});

describe("a diagram's box", () => {
	test("keeps its size before and after it draws: a row far up the thread is measured with its drawing in it, comes to the page already drawn, and moves nothing when its picture is made again", async () => {
		await open(withLine(lines(1, 60), diagram("d5", 5)));
		// Not measured while only its source shows: it waits off to the side,
		// and stands at the guess every unmeasured row has, not at its source's height.
		expect(standing(probeOf("d5"))).toMatchObject({ what: "source", unsized: true });
		expect(scroller().scrollHeight).toBe(60 * ROW);

		await after(A_TURN);
		await arrive();
		await after(A_TURN);
		// Drawn at rest, it is measured as its drawing and leaves the side.
		expect(rendered.length).toBe(1);
		expect(probeOf("d5")).toBeNull();
		expect(container.querySelector(".tape-probe")).toBeNull();
		const whole = 59 * ROW + AS_DRAWING;
		expect(scroller().scrollHeight).toBe(whole);

		// Scrolled to: the picture is in the row in the render that brings it, at the drawing's own size.
		await scrollTo(3 * ROW);
		expect(standing(rowOf("d5"))).toEqual({ what: "picture", style: "width: 300px; aspect-ratio: 300 / 120;", unsized: false });
		expect(scroller().scrollHeight).toBe(whole);
		expect(scroller().scrollTop).toBe(3 * ROW);
		expect(rendered.length).toBe(1);
		const reading = topOf("e7");

		// Another palette: the picture is let go, and an empty box of the same size stands in for it.
		await act(async () => {
			document.documentElement.dataset["theme"] = "light";
		});
		await turns();
		expect(standing(rowOf("d5"))).toEqual({ what: "box", style: "width: 300px; aspect-ratio: 300 / 120;", unsized: false });
		expect(scroller().scrollHeight).toBe(whole);
		// Drawn again at rest, in the same box.
		await act(async () => nearby());
		await after(A_TURN);
		expect(rendered.length).toBe(2);
		expect(standing(rowOf("d5"))).toEqual({ what: "picture", style: "width: 300px; aspect-ratio: 300 / 120;", unsized: false });
		expect(scroller().scrollHeight).toBe(whole);
		expect(scroller().scrollTop).toBe(3 * ROW);
		expect(topOf("e7")).toBe(reading);
	});

	test("drawn while its row is in the page above the line being read leaves that line where it was", async () => {
		await open(withLine(lines(1, 60), diagram("d50", 50)));
		// Up to where the diagram's row is in the page, above the view.
		wheel();
		await scrollTo(scroller().scrollTop - 4 * ROW);
		expect(standing(rowOf("d50"))).toMatchObject({ what: "source", unsized: true });
		expect(topOf("d50")).toBeLessThan(0);
		const reading = [...container.querySelectorAll<HTMLElement>(".tape-column > .tape-row")].find((row) => row.getBoundingClientRect().top >= 0)!;
		const at = reading.getBoundingClientRect().top;
		const row = rowOf("d50")!;

		await after(A_TURN);
		await arrive();
		await after(A_TURN);
		// The row is its drawing's height now, shorter than its source was, and
		// the line being read is where it stood: put back in the render that
		// drew the diagram, on an engine that anchors nothing itself.
		expect(standing(rowOf("d50")).what).toBe("picture");
		expect(row.getBoundingClientRect().height).toBe(AS_DRAWING);
		expect(reading.getBoundingClientRect().top).toBe(at);
		// The engine then says the row's box changed: nothing more moves.
		await act(async () => {
			for (const watcher of watchers) watcher.fire(row as unknown as Element);
		});
		await frame();
		expect(reading.getBoundingClientRect().top).toBe(at);
		// And that is the height its gap has once it leaves the page.
		await scrollTo(0);
		expect(rowOf("d50")).toBeNull();
		expect(scroller().scrollHeight).toBe(59 * ROW + AS_DRAWING);
	});

	test("drawn in view while the conversation is followed leaves the view at its foot in the frame that shows it, though the diagram is taller than its source was", async () => {
		grows = { source: 100, drawing: 400 };
		await open(withLine(lines(1, 60), diagram("d58", 58)));
		const foot = () => scroller().scrollHeight - VIEW;
		expect(scroller().scrollTop).toBe(foot());
		expect(topOf("d58")).toBeGreaterThan(0);
		await after(A_TURN);
		await arrive();
		// The drawing, and no frame since: the transcript's own pin has not had its turn.
		await after(A_TURN, () => rendered.length === 1);
		expect(standing(rowOf("d58")).what).toBe("picture");
		expect(scroller().scrollHeight).toBe(59 * ROW + 400);
		expect(scroller().scrollTop).toBe(foot());
		expect(container.querySelector(".jump-latest")).toBeNull();
		// And it is still followed once frames are drawn.
		await frame();
		await frame();
		expect(scroller().scrollTop).toBe(foot());
		expect(container.querySelector(".jump-latest")).toBeNull();
	});

	test("of a source that does not parse is its source with a line saying so, measured as that and not tried again", async () => {
		await open(withLine(lines(1, 60), diagram("d5", 5, "300x120", "graph TD\n  broken")));
		await after(A_TURN);
		await arrive();
		await after(A_TURN);
		// Settled as its source: measured, and off the side.
		expect(container.querySelector(".tape-probe")).toBeNull();
		expect(scroller().scrollHeight).toBe(59 * ROW + AS_SOURCE);
		await scrollTo(3 * ROW);
		expect(standing(rowOf("d5"))).toMatchObject({ what: "source", unsized: false });
		expect(rowOf("d5")?.querySelector(".mermaid-note")).not.toBeNull();
		await act(async () => nearby());
		await after(3 * A_TURN);
		expect(rendered).toEqual([]);
		expect(scroller().scrollHeight).toBe(59 * ROW + AS_SOURCE);
	});
});

describe("diagrams waiting to be drawn", () => {
	test("are drawn one to a turn with the window at rest between, the one near the view first, and none under a scroll", async () => {
		const events = withLine(withLine(withLine(lines(1, 60), diagram("d5", 5)), diagram("d9", 9)), diagram("d59", 59));
		await open(events);
		expect(rowOf("d59")).not.toBeNull();
		await after(A_TURN);
		await arrive();
		// The script's arrival is a turn of its own: nothing is drawn in it.
		expect(rendered).toEqual([]);
		await after(350);
		// The one on screen, though the two far up the thread asked as soon as it did.
		expect(rendered.length).toBe(1);
		expect(rendered[0]).toContain(" d59\n");
		// Scrolling: the next waits, however long.
		const foot = scroller().scrollTop;
		for (let step = 1; step <= 20; step++) {
			await scrollTo(foot - (step % 2) * 40);
			await after(100);
		}
		expect(rendered.length).toBe(1);
		// The last scroll, and the rest counted from it: not before 300 ms are up.
		await scrollTo(foot);
		await after(290);
		expect(rendered.length).toBe(1);
		await after(20);
		expect(rendered.length).toBe(2);
		// And the window gets the same rest after one drawing before the next.
		await after(270);
		expect(rendered.length).toBe(2);
		await after(40);
		expect(rendered.length).toBe(3);
	});

	test("are not drawn for a thread that was left before their turn came", async () => {
		await open(withLine(withLine(lines(1, 60), diagram("d5", 5)), diagram("d9", 9)));
		await after(A_TURN);
		expect(scriptTag()).not.toBeNull();
		// Another teammate's thread, with one diagram of its own, before the script has arrived.
		await act(async () => draw([...lines(1, 4), diagram("g5", 5)], `${ada}-grace`));
		await act(async () => nearby());
		await after(A_TURN);
		await arrive();
		await after(4 * A_TURN);
		expect(rendered.length).toBe(1);
		expect(rendered[0]).toContain(" g5\n");
	});

	test("a streamed block is not asked for until its source has stopped changing", async () => {
		const streamed = (body: string): TranscriptEvent => ({ kind: "agent", id: "d9", ts: 9, text: `\`\`\`mermaid\n%% 300x120 ${ada} d9\n${body}` }) as TranscriptEvent;
		Object.assign(globalThis, { mermaid });
		await act(async () => draw([...lines(1, 8), streamed("graph TD")]));
		await act(async () => nearby());
		// A few more characters every 200 ms, at rest otherwise: no prefix is drawn.
		for (const body of ["graph TD\n  A-->B\n  A-->", "graph TD\n  A-->B\n  A-->C\n  B", "graph TD\n  A-->B\n  A-->C\n  B-->C"]) {
			await after(200);
			await act(async () => draw([...lines(1, 8), streamed(body)]));
		}
		expect(rendered).toEqual([]);
		await after(A_TURN);
		expect(rendered.length).toBe(1);
		expect(rendered[0]).toContain("B-->C");
	});
});

/** The person's hand on the wheel: the scrolls that follow within a moment are theirs. */
function wheel() {
	scroller().dispatchEvent(new window.Event("wheel") as unknown as Event);
}
