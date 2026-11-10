import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";
import { layOut } from "./transcript-layout";

// Only the rows near the view are in the page (task-411). What that must not
// cost: the row being read stays where it was whatever happens above it, and
// everything a person can do to a row still finds the row there.

const ROW = 100;
const VIEW = 500;

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let Transcript: typeof import("../src/components/Transcript").Transcript;
let frames: FrameRequestCallback[];
let tall: Map<string, number>;
let restore: () => void;
let column: { width: number };
let scrollsIntoView: { id: string | undefined; options: ScrollIntoViewOptions | undefined }[];
let watchers: Watcher[];
/** A teammate of this test's own: a thread's row heights are remembered by thread, across conversations. */
let persona: string;
let made = 0;

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
	/** One notice for every watched box among these, as the engine gives one a frame. */
	fire(...targets: Element[]) {
		const entries = targets.filter((target) => this.targets.has(target)).map((target) => ({ target }) as ResizeObserverEntry);
		if (entries.length > 0) this.callback(entries, this as unknown as ResizeObserver);
	}
}

beforeEach(async () => {
	frames = [];
	watchers = [];
	const installed = installDom({
		requestAnimationFrame: (callback: FrameRequestCallback) => frames.push(callback),
		cancelAnimationFrame: () => {},
		ResizeObserver: Watcher,
	});
	teardown = installed.teardown;
	// An engine that does not anchor the scroll (WebKitGTK): what holds the view is the transcript's own.
	Object.assign(globalThis, { MutationObserver: installed.dom.MutationObserver, CSS: { escape: installed.dom.CSS.escape, supports: () => false } });
	persona = `ada-${++made}`;
	({ tall, column, scrollsIntoView, restore } = layOut({ row: ROW, view: VIEW }));
	act = (await import("react")).act;
	({ Transcript } = await import("../src/components/Transcript"));
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	restore();
	await teardown();
});

function lines(from: number, to: number): TranscriptEvent[] {
	const made: TranscriptEvent[] = [];
	for (let n = from; n <= to; n++) made.push({ kind: n % 2 ? "agent" : "user", id: `e${n}`, ts: n, text: `Line ${n}` } as TranscriptEvent);
	return made;
}

type Extra = Partial<Parameters<typeof Transcript>[0]>;

function draw(events: TranscriptEvent[], extra: Extra = {}) {
	root.render(<Transcript personaId={persona} name="Ada" events={events} streaming={[]} live={false} focus={null} {...extra} />);
}

const scroller = () => container.querySelector(".scroller") as unknown as HTMLElement;
const rowIds = () => [...container.querySelectorAll<HTMLElement>(".tape-column > .tape-row")].map((row) => row.dataset["eventId"]!);
const rowOf = (id: string) => container.querySelector(`.tape-column > [data-event-id="${id}"]`) as unknown as HTMLElement | null;
/** Where a row stands in the view: 0 is the view's top edge. */
const topOf = (id: string) => rowOf(id)?.getBoundingClientRect().top;
const elements = () => container.querySelectorAll("*").length;
/** The first row with any of itself in the view. */
const reading = () => rowIds().find((id) => topOf(id)! + (tall.get(id) ?? ROW) > 0 && topOf(id)! < VIEW)!;

/** The frames asked for, run. */
async function frame() {
	await act(async () => {
		const due = frames;
		frames = [];
		for (const one of due) one(0);
	});
}

/** A scroll to a place, and the frame the transcript answers it on. */
async function scrollTo(top: number) {
	scroller().scrollTop = top;
	await act(async () => {
		scroller().dispatchEvent(new window.Event("scroll") as unknown as Event);
	});
	await frame();
}

/** The person's hand on the wheel: the scrolls that follow within a moment are theirs. */
function wheel() {
	scroller().dispatchEvent(new window.Event("wheel") as unknown as Event);
}

/** One pass of the measuring that runs in the background: the two frames between one batch of rows and the next. */
async function pass() {
	await frame();
	await frame();
}

/** Every row measured: the background passes run until none is left to draw. */
async function measured() {
	for (let turn = 0; turn < 400; turn++) {
		await pass();
		if (container.querySelector(".tape-probe") === null && frames.length === 0) return;
	}
	throw new Error("the rows were never all measured");
}

/** The thread opened and at rest: drawn, following its last line, the first frame taken in. */
async function open(events: TranscriptEvent[], extra: Extra = {}) {
	await act(async () => draw(events, extra));
	await scrollTo(scroller().scrollHeight);
}

describe("a long thread", () => {
	test("keeps a bounded number of rows in the page, however many are loaded", async () => {
		// The last 372 lines, then the four pages above them: 1,972 rows.
		let events = lines(1601, 1972);
		const redraw = (more: boolean) => draw(events, { more, onEarlier: earlier });
		async function earlier() {
			const first = Number(events[0]!.id.slice(1));
			events = [...lines(first - 400, first - 1), ...events];
			redraw(first - 400 > 1);
		}
		await act(async () => redraw(true));
		await scrollTo(scroller().scrollHeight);
		await measured();
		// A view of five rows and what is kept past its edges, in the middle of what is loaded.
		await scrollTo(186 * ROW);
		const before = { rows: rowIds().length, elements: elements() };
		expect(before.rows).toBeLessThanOrEqual(30);

		// To the top, as often as there is a page above.
		for (let page = 0; page < 4; page++) {
			await scrollTo(0);
			await measured();
			expect(rowIds().length).toBeLessThanOrEqual(30);
		}
		expect(scroller().scrollHeight).toBe(1972 * ROW);
		// The whole thread is loaded, and the page holds no more than it did with a fifth of it.
		for (const top of [0, 986 * ROW, scroller().scrollHeight]) {
			await scrollTo(top);
			expect(rowIds().length).toBeLessThanOrEqual(before.rows);
			expect(elements()).toBeLessThanOrEqual(before.elements);
		}
		expect(rowIds().at(-1)).toBe("e1972");
	});

	test("a thread a few rows long opens on its last line", async () => {
		for (let n = 1; n <= 15; n++) tall.set(`e${n}`, 150);
		await act(async () => draw(lines(1, 15)));
		await frame();
		expect(scroller().scrollTop).toBe(15 * 150 - VIEW);
		expect(container.querySelector(".jump-latest")).toBeNull();
	});

	test("opens on its last lines, and draws the rows near the view after a scroll", async () => {
		await open(lines(1, 300));
		expect(rowIds().at(-1)).toBe("e300");
		expect(rowOf("e1")).toBeNull();
		wheel();
		await scrollTo(100 * ROW);
		expect(reading()).toBe("e101");
		expect(rowOf("e300")).toBeNull();
		// Past each edge of the view there are rows, not a gap.
		expect(topOf(rowIds()[0]!)).toBeLessThanOrEqual(-400);
		expect(topOf(rowIds().at(-1)!)).toBeGreaterThanOrEqual(VIEW + 400);
	});
});

describe("the row being read stays where it was", () => {
	/** Rows of several heights, none of them the guess. */
	function uneven(count: number) {
		for (let n = 1; n <= count; n++) tall.set(`e${n}`, n % 7 === 0 ? 0 : n % 5 === 0 ? 340 : n % 3 === 0 ? 40 : 100);
	}

	test("when a row drawn for the first time above it is not as tall as was guessed", async () => {
		uneven(200);
		await open(lines(1, 200));
		// Up the thread before anything above has been measured.
		for (let step = 0; step < 60; step++) {
			const at = scroller().scrollTop;
			const row = reading();
			const was = topOf(row)!;
			wheel();
			await scrollTo(at - 150);
			// It moved down the view by what was scrolled, and by nothing else.
			expect(topOf(row)! - was).toBe(150);
		}
	});

	test("when rows above it are measured in the background", async () => {
		uneven(200);
		await open(lines(1, 200));
		wheel();
		await scrollTo(scroller().scrollTop - 900);
		const row = reading();
		const was = topOf(row)!;
		for (let turn = 0; turn < 30; turn++) {
			await pass();
			expect(topOf(row)).toBe(was);
		}
		await measured();
		expect(topOf(row)).toBe(was);
		// Every row's height is known now: the column is as tall as its rows.
		let total = 0;
		for (let n = 1; n <= 200; n++) total += tall.get(`e${n}`)!;
		expect(scroller().scrollHeight).toBe(total);
	});

	test("rows are not measured while a font is on its way: their words would wrap where they will not", async () => {
		const fonts = { status: "loading", listeners: [] as (() => void)[], addEventListener(_: string, listener: () => void) { this.listeners.push(listener); }, removeEventListener() {} };
		Object.defineProperty(document, "fonts", { configurable: true, value: fonts });
		// In the font standing in, every row is a line taller.
		for (let n = 1; n <= 120; n++) tall.set(`e${n}`, 130);
		await open(lines(1, 120));
		for (let turn = 0; turn < 10; turn++) await pass();
		// Rows wait off to the side, and none of their heights has been taken: the gap is still the guess.
		expect(container.querySelectorAll(".tape-probe > .tape-row").length).toBeGreaterThan(0);
		const guessed = scroller().scrollHeight;
		// The font arrives and the rows are their own height.
		for (let n = 1; n <= 120; n++) tall.set(`e${n}`, 100);
		fonts.status = "loaded";
		await act(async () => {
			for (const listener of fonts.listeners) listener();
		});
		await measured();
		expect(scroller().scrollHeight).toBe(120 * 100);
		expect(guessed).not.toBe(120 * 130);
	});

	test("within a pixel on an engine that keeps the scroll in whole pixels, however many heights are learned above it: what one put-back cannot give is carried to the next, so the fractions never add up", async () => {
		// The layout again, as WebKit scrolls it; and rows that stand at fractions of a pixel, as text does.
		restore();
		({ tall, column, scrollsIntoView, restore } = layOut({ row: ROW, view: VIEW, whole: true }));
		for (let n = 1; n <= 400; n++) tall.set(`e${n}`, 60 + ((n * 37) % 64) / 64 + (n % 9) * 11);
		await open(lines(1, 400));
		wheel();
		await scrollTo(scroller().scrollTop - 900);
		const row = reading();
		const was = topOf(row)!;
		// Every pass learns twelve heights above the view, and each moves the
		// row by a fraction the scroll cannot take. Half a pixel may already
		// be owed when the row is first looked at here, and half a pixel may
		// be owed at any look after: never more than one between the two.
		let learned = 0;
		for (let turn = 0; turn < 40; turn++) {
			const before = scroller().scrollHeight;
			await pass();
			if (scroller().scrollHeight !== before) learned++;
			expect(Math.abs(topOf(row)! - was)).toBeLessThanOrEqual(1);
		}
		expect(learned).toBeGreaterThan(10);
		await measured();
		expect(Math.abs(topOf(row)! - was)).toBeLessThanOrEqual(1);
	});

	test("when a row above it grows by itself", async () => {
		await open(lines(1, 200));
		await measured();
		await scrollTo(100 * ROW);
		const row = reading();
		const above = rowOf("e97")!;
		expect(topOf("e97")).toBeLessThan(0);
		const at = scroller().scrollTop;
		// A picture arrives in a row above the view: 80px taller, and the engine says so.
		tall.set("e97", ROW + 80);
		await act(async () => {
			for (const watcher of watchers) watcher.fire(above as unknown as Element);
		});
		expect(scroller().scrollTop).toBe(at + 80);
		expect(topOf(row)).toBe(0);
		// The height it grew to is the height its gap has once it leaves the page.
		await scrollTo(scroller().scrollHeight);
		expect(rowOf("e97")).toBeNull();
		expect(scroller().scrollHeight).toBe(200 * ROW + 80);
	});

	test("and is not put back where it stood once the view has been moved on: a pin's scroll, or the person's, is not rows having moved", async () => {
		await open(lines(1, 200));
		await measured();
		wheel();
		await scrollTo(100 * ROW);
		const above = rowOf("e97")!;
		// The view is taken further down, as a pin to the bottom takes it, and on
		// that same frame a row that was above it grows: nothing was being read
		// where the row on screen used to stand.
		scroller().scrollTop = 103 * ROW;
		tall.set("e97", ROW + 80);
		await act(async () => {
			for (const watcher of watchers) watcher.fire(above as unknown as Element);
		});
		expect(scroller().scrollTop).toBe(103 * ROW);
	});

	test("when the window is made narrower: every row is measured again at the new width", async () => {
		await open(lines(1, 200));
		await measured();
		wheel();
		await scrollTo(100 * ROW);
		expect(topOf("e101")).toBe(0);
		// Narrower, every line wraps once more: each row is 40px taller, in the page or not.
		for (let n = 1; n <= 200; n++) tall.set(`e${n}`, ROW + 40);
		column.width = 500;
		await act(async () => {
			for (const watcher of watchers) watcher.fire(...watcher.targets);
		});
		await frame();
		// The line being read is where it was, whatever the rows above it are now taken to be.
		expect(topOf("e101")).toBe(0);
		await measured();
		expect(topOf("e101")).toBe(0);
		expect(scroller().scrollHeight).toBe(200 * (ROW + 40));
	});

	test("when a page of older rows lands after the window has moved on", async () => {
		let events = lines(201, 400);
		let land: () => void = () => {};
		const redraw = (more: boolean) => draw(events, { more, onEarlier: earlier });
		function earlier() {
			return new Promise<void>((done) => {
				land = () => {
					events = lines(1, 400);
					redraw(false);
					done();
				};
			});
		}
		await act(async () => redraw(true));
		await scrollTo(scroller().scrollHeight);
		await measured();
		// Near the top, which asks for the page above; then back down the thread while it is on its way.
		await scrollTo(3 * ROW);
		expect(scroller().dataset["landing"]).toBe("");
		await scrollTo(60 * ROW);
		expect(reading()).toBe("e261");
		await act(async () => land());
		await frame();
		// Two hundred rows arrived above: the same line is under the eye.
		expect(reading()).toBe("e261");
		expect(topOf("e261")).toBe(0);
		expect(scroller().dataset["landing"]).toBeUndefined();
	});

	test("while a reply streams and the conversation is followed", async () => {
		let events = lines(1, 120);
		await open(events);
		for (let n = 121; n <= 150; n++) {
			events = [...events, ...lines(n, n)];
			await act(async () => draw(events));
			// The column grew: the engine says so, and the frame after pins.
			await act(async () => {
				for (const watcher of watchers) watcher.fire(scroller().firstElementChild as unknown as Element);
			});
			await frame();
			// The line that arrived is drawn and the view is on it.
			expect(rowIds().at(-1)).toBe(`e${n}`);
			expect(scroller().scrollTop).toBe(scroller().scrollHeight - VIEW);
		}
		expect(container.querySelector(".jump-latest")).toBeNull();
		expect(rowIds().length).toBeLessThanOrEqual(30);
	});

	test("while a reply streams and the reader has left the bottom", async () => {
		let events = lines(1, 120);
		await open(events);
		await measured();
		wheel();
		await scrollTo(40 * ROW);
		expect(container.querySelector(".jump-latest")).not.toBeNull();
		const drawn = rowIds();
		for (let n = 121; n <= 150; n++) {
			events = [...events, ...lines(n, n)];
			await act(async () => draw(events));
			await act(async () => {
				for (const watcher of watchers) watcher.fire(scroller().firstElementChild as unknown as Element);
			});
			await frame();
			expect(scroller().scrollTop).toBe(40 * ROW);
			expect(topOf("e41")).toBe(0);
		}
		// What arrived is far below the view: none of it is in the page.
		expect(rowIds()).toEqual(drawn);
	});

	test("on switching teammate and back: the conversation opens on its last line, its rows as tall as they were", async () => {
		uneven(200);
		let total = 0;
		for (let n = 1; n <= 200; n++) total += tall.get(`e${n}`)!;
		await act(async () => root.render(<Transcript key="ada" personaId={persona} name="Ada" events={lines(1, 200)} streaming={[]} live={false} focus={null} />));
		await scrollTo(scroller().scrollHeight);
		await measured();
		await scrollTo(50 * ROW);
		// Another teammate, then back: a conversation is drawn afresh each time.
		await act(async () => root.render(<Transcript key="grace" personaId="grace" name="Grace" events={[{ kind: "user", id: "g1", ts: 1, text: "Hello" } as TranscriptEvent]} streaming={[]} live={false} focus={null} />));
		await act(async () => root.render(<Transcript key="ada" personaId={persona} name="Ada" events={lines(1, 200)} streaming={[]} live={false} focus={null} />));
		await frame();
		expect(rowIds().at(-1)).toBe("e200");
		expect(scroller().scrollTop).toBe(total - VIEW);
		expect(container.querySelector(".jump-latest")).toBeNull();
		// Nothing is at a guess: the column is as tall as it was, before any row is measured again.
		expect(scroller().scrollHeight).toBe(total);
	});
});

describe("what a person does to a row finds it in the page", () => {
	test("a selection across rows keeps them in the page when the view moves away, and lets them go when it is cleared", async () => {
		await open(lines(1, 300));
		await measured();
		await scrollTo(100 * ROW);
		// From the middle of line 102 to the middle of line 104.
		const from = rowOf("e102")!.querySelector(".speech")!.firstChild!;
		const to = rowOf("e104")!.querySelector(".speech")!.firstChild!;
		const selection = document.getSelection()!;
		const range = document.createRange();
		range.setStart(from as unknown as Node, 2);
		range.setEnd(to as unknown as Node, 4);
		selection.removeAllRanges();
		selection.addRange(range);
		await act(async () => {
			document.dispatchEvent(new window.Event("selectionchange") as unknown as Event);
		});
		await frame();
		const copied = selection.toString();
		expect(copied).toContain("Line 103");

		await scrollTo(250 * ROW);
		// Far from the view, the rows it covers are still there, and so is what it would copy.
		expect(rowOf("e102")).not.toBeNull();
		expect(rowOf("e103")).not.toBeNull();
		expect(rowOf("e104")).not.toBeNull();
		expect(rowOf("e101")).toBeNull();
		expect(rowOf("e105")).toBeNull();
		expect(selection.toString()).toBe(copied);

		selection.removeAllRanges();
		await act(async () => {
			document.dispatchEvent(new window.Event("selectionchange") as unknown as Event);
		});
		await frame();
		expect(rowOf("e103")).toBeNull();
	});

	test("a quoted line far from the view is brought into the page, scrolled to the middle and lit", async () => {
		const reply = { kind: "user", id: "e301", ts: 301, text: "About that", replyTo: "e5" } as TranscriptEvent;
		await open([...lines(1, 300), reply]);
		await measured();
		expect(rowOf("e5")).toBeNull();
		const quote = rowOf("e301")!.querySelector(".quote") as unknown as HTMLElement;
		expect(quote.textContent).toContain("Line 5");
		await act(async () => quote.click());
		await frame();
		expect(scrollsIntoView.at(-1)).toEqual({ id: "e5", options: { block: "center", behavior: "smooth" } });
		expect(rowOf("e5")!.classList.contains("row-lit")).toBe(true);
		// In the middle of the view, with its neighbours drawn around it.
		expect(topOf("e5")).toBe((VIEW - ROW) / 2);
		expect(rowOf("e4")).not.toBeNull();
		expect(rowOf("e6")).not.toBeNull();
		expect(rowOf("e300")).toBeNull();
		// The jump let go of the bottom: the way back down is offered.
		expect(container.querySelector(".jump-latest")).not.toBeNull();
	});

	test("a search hit far from the view is brought into the page, scrolled to the middle and lit", async () => {
		const events = lines(1, 300);
		await open(events);
		await measured();
		expect(rowOf("e40")).toBeNull();
		await act(async () => draw(events, { focus: { eventId: "e40", at: 1 } }));
		await frame();
		expect(scrollsIntoView.at(-1)).toEqual({ id: "e40", options: { block: "center", behavior: "smooth" } });
		expect(rowOf("e40")!.classList.contains("row-lit")).toBe(true);
		expect(topOf("e40")).toBe((VIEW - ROW) / 2);
	});

	test("a jump ends where its row is, though the rows on the way were not as tall as was guessed", async () => {
		for (let n = 1; n <= 300; n++) tall.set(`e${n}`, n % 2 ? 60 : 180);
		const events = lines(1, 300);
		await open(events);
		// Nothing above the window has been measured: the row is scrolled to where the guesses put it.
		await act(async () => draw(events, { focus: { eventId: "e40", at: 1 } }));
		await frame();
		// The scroll ends, and the row is put in the middle of what is drawn around it now.
		await act(async () => {
			scroller().dispatchEvent(new window.Event("scrollend") as unknown as Event);
		});
		await frame();
		expect(topOf("e40")).toBe((VIEW - 180) / 2);
	});

	test("a chapter mark far up the thread is found by a jump, and reads the same when its row comes back", async () => {
		const chapter = { kind: "chapter", id: "c1", ts: 50, title: "Billing" } as TranscriptEvent;
		const events = [...lines(1, 50), chapter, ...lines(51, 300)];
		await open(events);
		await measured();
		expect(container.querySelector(".chapter-rule")).toBeNull();
		await act(async () => draw(events, { focus: { eventId: "c1", at: 1 } }));
		await frame();
		expect(rowOf("c1")!.querySelector(".chapter-title")!.textContent).toBe("Billing");
		expect(topOf("c1")).toBe((VIEW - ROW) / 2);
		// A chapter is not stamped, and neither is the line under it.
		expect(rowOf("c1")!.querySelector(".tape-stamp")).toBeNull();
		expect(rowOf("e51")!.querySelector(".tape-stamp")).toBeNull();
	});

	test("the row on screen has the next and previous rows in the page, however tall it is, so a screen reader can move to them", async () => {
		// One row taller than everything the window reaches for.
		tall.set("e150", 4_000);
		await open(lines(1, 300));
		await measured();
		// Previous, row by row, each scrolled into view as VoiceOver does on moving to it.
		let at = rowOf("e300")!;
		for (let step = 0; step < 299; step++) {
			const previous = at.previousElementSibling as unknown as HTMLElement | null;
			expect(previous?.className).toBe("tape-row");
			await act(async () => previous!.scrollIntoView({ block: "nearest" }));
			await frame();
			expect(previous!.isConnected).toBe(true);
			at = previous!;
		}
		expect(at.dataset["eventId"]).toBe("e1");
		expect(at.previousElementSibling).toBeNull();
		// And next, back down to the last line.
		for (let step = 0; step < 299; step++) {
			const next = at.nextElementSibling as unknown as HTMLElement | null;
			expect(next?.className).toBe("tape-row");
			await act(async () => next!.scrollIntoView({ block: "nearest" }));
			await frame();
			at = next!;
		}
		expect(at.dataset["eventId"]).toBe("e300");
	});

	test("a row the person pressed stays in the page, so what they opened stays open", async () => {
		const scheduled = { kind: "user", id: "s1", ts: 100, text: "Check the renewals", scheduled: { jobId: "j1", name: "Renewals" } } as TranscriptEvent;
		await open([...lines(1, 100), scheduled, ...lines(101, 300)]);
		await measured();
		await act(async () => draw([...lines(1, 100), scheduled, ...lines(101, 300)], { focus: { eventId: "s1", at: 1 } }));
		await frame();
		const pill = rowOf("s1")!.querySelector("button[aria-expanded]") as unknown as HTMLElement;
		expect(pill.getAttribute("aria-expanded")).toBe("false");
		await act(async () => pill.click());
		expect(rowOf("s1")!.textContent).toContain("Check the renewals");

		// Another jump takes the view, and this row's place as a jump's target, far away.
		await act(async () => draw([...lines(1, 100), scheduled, ...lines(101, 300)], { focus: { eventId: "e290", at: 2 } }));
		await frame();
		await scrollTo(scroller().scrollHeight);
		expect(rowOf("e101")).toBeNull();
		expect(rowOf("s1")!.querySelector("button[aria-expanded]")!.getAttribute("aria-expanded")).toBe("true");
		expect(rowOf("s1")!.textContent).toContain("Check the renewals");
	});

	test("the row that holds the keyboard's focus stays in the page", async () => {
		await open(lines(1, 300), { onReply: () => {} });
		await measured();
		await scrollTo(100 * ROW);
		const bubble = rowOf("e103")!.querySelector(".speech") as unknown as HTMLElement;
		bubble.focus();
		expect(document.activeElement as unknown).toBe(bubble);
		await scrollTo(250 * ROW);
		expect(rowOf("e102")).toBeNull();
		expect(rowOf("e103")).not.toBeNull();
		expect(document.activeElement as unknown).toBe(bubble);
	});

	test("the scrollbar's thumb dragged up from the foot is the person's scroll, not thrown back to the bottom", async () => {
		await open(lines(1, 300));
		const bottom = scroller().scrollHeight - VIEW;
		const thumb = container.querySelector(".scroll-thumb") as unknown as HTMLElement;
		expect(thumb).not.toBeNull();
		// Held and dragged, while a row above the view grows in the same frame: a new layout.
		await act(async () => {
			thumb.dispatchEvent(new window.PointerEvent("pointerdown", { bubbles: true, buttons: 1 }) as unknown as Event);
		});
		scroller().scrollTop = bottom - 3 * ROW;
		tall.set("e293", ROW + 30);
		await act(async () => {
			scroller().dispatchEvent(new window.Event("scroll") as unknown as Event);
		});
		await frame();
		expect(scroller().scrollTop).toBe(bottom - 3 * ROW);
		expect(container.querySelector(".jump-latest")).not.toBeNull();
	});

	test("the way back down to the latest line goes there and follows again", async () => {
		await open(lines(1, 300));
		wheel();
		await scrollTo(100 * ROW);
		const asked: ScrollToOptions[] = [];
		// The engine's glide, done at once: it arrives and says the scroll has ended.
		scroller().scrollTo = ((options: ScrollToOptions) => {
			asked.push(options);
			scroller().scrollTop = options.top!;
			scroller().dispatchEvent(new window.Event("scroll") as unknown as Event);
			scroller().dispatchEvent(new window.Event("scrollend") as unknown as Event);
		}) as typeof window.scrollTo;
		const down = container.querySelector(".jump-latest") as unknown as HTMLElement;
		await act(async () => down.click());
		await frame();
		await scrollTo(scroller().scrollTop);
		expect(asked).toEqual([{ top: 300 * ROW, behavior: "smooth" }]);
		expect(scroller().scrollTop).toBe(300 * ROW - VIEW);
		expect(rowIds().at(-1)).toBe("e300");
		expect(container.querySelector(".jump-latest")).toBeNull();
	});
});

describe("the page while the window moves", () => {
	test("the column holds the room its rows take in all, whichever of them are in the page", async () => {
		for (let n = 1; n <= 300; n++) tall.set(`e${n}`, n % 2 ? 60 : 180);
		await open(lines(1, 300));
		await measured();
		const room = container.querySelector(".tape-column > .tape-room") as unknown as HTMLElement;
		expect(room.style.height).toBe(`${150 * 60 + 150 * 180}px`);
		wheel();
		await scrollTo(120 * 120);
		expect(room.style.height).toBe(`${150 * 60 + 150 * 180}px`);
	});

	test("the engine's own anchoring is off while rows are swapped for a gap, and on again once they are in", async () => {
		await open(lines(1, 300));
		await measured();
		wheel();
		await scrollTo(150 * ROW);
		expect("swapping" in scroller().dataset).toBe(false);
		// Whether it was off at the moment each row left the page.
		const column = container.querySelector(".tape-column") as unknown as HTMLElement;
		const taken: boolean[] = [];
		const remove = column.removeChild.bind(column);
		column.removeChild = ((child: Node) => {
			taken.push("swapping" in scroller().dataset);
			return remove(child);
		}) as typeof column.removeChild;
		await scrollTo(170 * ROW);
		expect(taken.length).toBeGreaterThan(0);
		expect(taken).not.toContain(false);
		expect("swapping" in scroller().dataset).toBe(false);
	});

	test("a jump has the engine's anchoring off from when it sets out until it arrives", async () => {
		const events = lines(1, 300);
		await open(events);
		await measured();
		await act(async () => draw(events, { focus: { eventId: "e40", at: 1 } }));
		await frame();
		// On its way: every swap under it leaves the anchoring alone.
		expect("swapping" in scroller().dataset).toBe(true);
		await act(async () => {
			scroller().dispatchEvent(new window.Event("scrollend") as unknown as Event);
		});
		await frame();
		expect("swapping" in scroller().dataset).toBe(false);
		expect(topOf("e40")).toBe((VIEW - ROW) / 2);
	});

	test("a picture already read is drawn in the render that brings its row back, not after a placeholder", async () => {
		const { wire } = await import("../src/wire");
		const reads = spyOn(wire, "command").mockImplementation((async (name: string) => {
			if (name === "file.read") return { mimeType: "image/png", data: "iVBORw0KGgo=" };
			return null;
		}) as typeof wire.command);
		const picture = { kind: "image", name: "pass.png", path: "/tmp/pass.png", mimeType: "image/png", size: 8 } as const;
		const sent = { kind: "user", id: "p1", ts: 5, text: "", attachments: [picture] } as TranscriptEvent;
		const events = [...lines(1, 4), sent, ...lines(6, 300)];
		await open(events);
		await measured();
		// Up to it: the picture is read from the desk and drawn.
		await act(async () => draw(events, { focus: { eventId: "p1", at: 1 } }));
		await frame();
		for (let waited = 0; waited < 50 && rowOf("p1")!.querySelector("img") === null; waited++) await act(async () => new Promise((done) => setTimeout(done, 10)));
		const url = rowOf("p1")!.querySelector("img")!.getAttribute("src");
		expect(url).toStartWith("blob:");
		const asked = reads.mock.calls.filter(([name]) => name === "file.read").length;

		// Away, so its row leaves the page, and back.
		await act(async () => draw(events, { focus: { eventId: "e290", at: 2 } }));
		await frame();
		await scrollTo(scroller().scrollHeight);
		expect(rowOf("p1")).toBeNull();
		await act(async () => draw(events, { focus: { eventId: "p1", at: 3 } }));
		// Drawn with its picture at once: no placeholder, and nothing asked of the desk again.
		expect(rowOf("p1")!.querySelector(".sent-placeholder")).toBeNull();
		expect(rowOf("p1")!.querySelector("img")!.getAttribute("src")).toBe(url);
		expect(reads.mock.calls.filter(([name]) => name === "file.read").length).toBe(asked);
	});
});

describe("how tall a thread's rows are", () => {
	test("the rows to draw are those in the stretch asked for and one more at each end", async () => {
		const { rowsIn } = await import("../src/components/transcript/rowWindow");
		const ids = ["a", "b", "c", "d", "e", "f"];
		const heights = { known: new Map(ids.map((id) => [id, 100])), guess: 64, guessed: true, width: 0 };
		expect(rowsIn(ids, heights, 250, 350)).toEqual({ from: 1, to: 4 });
		expect(rowsIn(ids, heights, -500, 50)).toEqual({ from: 0, to: 1 });
		// Past the last row, the last row and the one before it.
		expect(rowsIn(ids, heights, 5_000, 6_000)).toEqual({ from: 4, to: 5 });
		expect(rowsIn(ids, heights, 280, 290)).toEqual({ from: 1, to: 3 });
		// A row never drawn stands at the guess: every row after it is that much higher up.
		heights.known.delete("a");
		expect(rowsIn(ids, heights, 280, 290)).toEqual({ from: 2, to: 4 });
	});

	test("the guess is the mean of the first rows measured, and is taken once", async () => {
		const { settleGuess } = await import("../src/components/transcript/rowWindow");
		const heights = { known: new Map<string, number>(), guess: 64, guessed: false, width: 0 };
		settleGuess(heights);
		expect(heights).toMatchObject({ guess: 64, guessed: false });
		heights.known.set("a", 30).set("b", 50);
		expect(settleGuess(heights)).toBe(true);
		expect(heights).toMatchObject({ guess: 40, guessed: true });
		heights.known.set("c", 400);
		expect(settleGuess(heights)).toBe(false);
		expect(heights.guess).toBe(40);
	});

	test("a thread's heights are kept while eight others have been shown since, and no longer", async () => {
		const { heightsOf } = await import("../src/components/transcript/rowWindow");
		const first = heightsOf("dm:kept-test");
		for (let n = 0; n < 7; n++) heightsOf(`dm:other-${n}`);
		expect(heightsOf("dm:kept-test")).toBe(first);
		for (let n = 0; n < 8; n++) heightsOf(`dm:more-${n}`);
		expect(heightsOf("dm:kept-test")).not.toBe(first);
	});
});
