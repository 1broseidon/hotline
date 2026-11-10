import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

// A capture of the computer's screen costs its thumbnail, not the capture
// (task-414). A capture is a whole screen and is drawn 110 px wide; drawn
// from the capture itself, all of it is decoded for as long as its row is
// in the page. So on macOS, where a copy made at the thumbnail's own pixels
// is the picture WebKit itself draws, the thumbnail is drawn from that
// copy, made while the window is at rest, and the capture itself only
// where it is asked for: pressed, and in the viewer.
//
// Time here is the test's (as in diagrams.test.tsx): the window's timers
// run when `after` says so. The engine's part is a stand-in: decoding a
// capture gives a picture of the size the test names, and how big a copy
// it was asked to scale one to is noted.

/** The capture every test here holds: a screen 1568 by 882. */
const SCREEN = { width: 1568, height: 882 };
const png = "data:image/png;base64,iVBORw0KGgo=";

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let ComputerFrame: typeof import("../src/components/transcript/ComputerFrame").ComputerFrame;
let frames: FrameRequestCallback[];
let timers: { id: number; at: number; run: () => void }[];
let now: number;
/** Every URL made, with what it is a URL of, and every one let go. */
let urls: Map<string, Blob>;
let revoked: string[];
/** Every capture decoded whole, and every copy made: how big, and by which of the engine's filters. */
let decoded: Blob[];
let drawn: { width: number; height: number; quality: string }[];
/** What the engine makes of a capture: its size, or a refusal. */
let screen: { width: number; height: number } | null;
/** Where the window runs: the shell says, and a copy is made only on macOS. */
let os: string;
/** The window's pixel ratio, and what is listening for it to change. */
let ratio: number;
let onRatio: (() => void)[];
let restore: () => void;
/** A mark in every event's id, so no test meets a copy another made: copies are kept by event, across tests. */
let mark = 0;

beforeEach(async () => {
	frames = [];
	timers = [];
	urls = new Map();
	revoked = [];
	decoded = [];
	drawn = [];
	screen = SCREEN;
	os = "macos";
	ratio = 1;
	onRatio = [];
	now = 1_000;
	mark++;
	let timerId = 0;
	/** An image, as far as a copy is kept by one: loaded as soon as it has a source. */
	class Loaded {
		src = "";
		async decode() {}
	}
	const installed = installDom({
		requestAnimationFrame: (callback: FrameRequestCallback) => frames.push(callback),
		cancelAnimationFrame: () => {},
		Image: Loaded,
		// The engine's decoder: a capture as it is, to learn its size, or scaled as it is decoded.
		createImageBitmap: async (blob: Blob, scaled?: ImageBitmapOptions) => {
			if (scaled === undefined) decoded.push(blob);
			if (screen === null) throw new Error("The source image could not be decoded.");
			if (scaled === undefined) return { ...screen, close() {} };
			drawn.push({ width: scaled.resizeWidth!, height: scaled.resizeHeight!, quality: scaled.resizeQuality! });
			return { width: scaled.resizeWidth, height: scaled.resizeHeight, close() {} };
		},
	});
	teardown = installed.teardown;
	Object.assign(installed.dom, {
		setTimeout: (run: () => void, ms = 0) => {
			timers.push({ id: ++timerId, at: now + ms, run });
			return timerId;
		},
		clearTimeout: (id: number) => {
			timers = timers.filter((timer) => timer.id !== id);
		},
		matchMedia: () => ({
			matches: true,
			addEventListener: (_kind: string, listener: () => void) => onRatio.push(listener),
			removeEventListener: (_kind: string, listener: () => void) => {
				onRatio = onRatio.filter((one) => one !== listener);
			},
		}),
	});
	Object.defineProperty(installed.dom, "devicePixelRatio", { configurable: true, get: () => ratio });
	Object.defineProperty(installed.dom, "__hotlineDesk", { configurable: true, get: () => ({ platform: os }) });
	spyOn(performance, "now").mockImplementation(() => now);
	// A canvas that takes what is drawn to it and makes a PNG that says its size.
	const canvas = (installed.dom as unknown as { HTMLCanvasElement: typeof HTMLCanvasElement }).HTMLCanvasElement.prototype;
	const before = { getContext: canvas.getContext, toBlob: canvas.toBlob };
	canvas.getContext = (() => ({ drawImage() {} })) as unknown as typeof canvas.getContext;
	canvas.toBlob = function (this: HTMLCanvasElement, done: BlobCallback) {
		done(new Blob([`${this.width}x${this.height}`], { type: "image/png" }));
	};
	const make = URL.createObjectURL;
	const revoke = URL.revokeObjectURL;
	URL.createObjectURL = (blob: Blob) => {
		const url = make(blob);
		urls.set(url, blob);
		return url;
	};
	URL.revokeObjectURL = (url: string) => {
		revoked.push(url);
		revoke(url);
	};
	restore = () => {
		Object.assign(canvas, before);
		URL.createObjectURL = make;
		URL.revokeObjectURL = revoke;
	};
	act = (await import("react")).act;
	({ ComputerFrame } = await import("../src/components/transcript/ComputerFrame"));
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	// What is still waiting for its copy is made, so the next test's line starts empty.
	await after(60_000);
	await act(async () => root.unmount());
	container.remove();
	restore();
	await teardown();
});

const id = (name: string) => `${name}-${mark}`;
const draw = (...names: string[]) => act(async () => root.render(names.map((name) => <ComputerFrame key={name} eventId={id(name)} dataUrl={png} />)));
const thumbnails = () => [...container.querySelectorAll(".picture-open img")] as unknown as HTMLImageElement[];
const thumbnail = () => thumbnails()[0]!;
/** What a URL is a URL of: the capture as it came, or a copy of it and how big. */
const whatIs = (url: string | null) => {
	const blob = urls.get(url ?? "");
	if (blob === undefined) return "nothing";
	return blob.size === 8 ? "the capture" : "a copy";
};
const copySize = async (url: string | null) => urls.get(url ?? "")?.text();

/** Turns of the real event loop, for what no timer of the window's carries: a promise's chain. */
async function turns() {
	for (let turn = 0; turn < 4; turn++) await act(async () => new Promise((done) => setTimeout(done, 0)));
}

/** Lets `ms` of the window's time pass: every timer due in it runs at its own moment, in order, and the frames each asks for are drawn. */
async function after(ms: number) {
	const end = now + ms;
	for (;;) {
		const due = timers.filter((timer) => timer.at <= end).sort((a, b) => a.at - b.at || a.id - b.id)[0];
		if (due === undefined) break;
		timers = timers.filter((timer) => timer !== due);
		now = Math.max(now, due.at);
		await act(async () => due.run());
		await turns();
		for (let pass = 0; pass < 6 && frames.length > 0; pass++) {
			await act(async () => {
				const asked = frames;
				frames = [];
				for (const one of asked) one(0);
			});
			await turns();
		}
	}
	now = end;
}

/** A press on something, as the pointer makes one. */
const press = (target: Element) => act(async () => target.dispatchEvent(new window.Event("pointerdown", { bubbles: true }) as unknown as Event));

describe("a thumbnail's decoded size is bounded by its drawn size", () => {
	test("once the window is at rest it is drawn from a copy of the capture as wide as its box, in the box the capture gave it, and the capture is let go", async () => {
		await draw("f");
		// First seen, it is drawn from the capture, as the engine decodes it: nothing smaller exists yet.
		const first = thumbnail().getAttribute("src");
		expect(whatIs(first)).toBe("the capture");
		expect(thumbnail().getAttribute("decoding")).toBe("async");
		expect(decoded).toEqual([]);

		await after(1_000);
		const img = thumbnail();
		expect(whatIs(img.getAttribute("src"))).toBe("a copy");
		// 110 px of thumbnail from 1,568 of screen, scaled by the engine's best filter, as it scales the capture.
		expect(drawn).toEqual([{ width: 110, height: 62, quality: "high" }]);
		expect(await copySize(img.getAttribute("src"))).toBe("110x62");
		// The box is still the capture's shape: 110 by 61.875, not the copy's 110 by 62.
		expect(img.style.aspectRatio).toBe("1568 / 882");
		expect(img.getAttribute("decoding")).toBe("sync");
		// The capture was decoded as it is once, to learn its size, and nothing holds it now.
		expect(decoded.length).toBe(1);
		expect(revoked).toEqual([first!]);
	});

	test("on a screen of two pixels to one the copy is twice as wide, and it is made again when the window moves to such a screen", async () => {
		await draw("f");
		await after(1_000);
		const one = thumbnail().getAttribute("src");
		expect(await copySize(one)).toBe("110x62");

		ratio = 2;
		await act(async () => {
			for (const listener of [...onRatio]) listener();
		});
		// Until the new copy is made the capture stands in, not a copy too coarse for the screen.
		expect(whatIs(thumbnail().getAttribute("src"))).toBe("the capture");
		await after(1_000);
		expect(await copySize(thumbnail().getAttribute("src"))).toBe("220x124");
		expect(revoked).toContain(one!);
	});

	test("a row that left before its copy was made is copied all the same, and coming back draws the copy in its first render", async () => {
		// As a row far up a thread is: drawn off to the side to be measured, and gone two frames later.
		await draw("f");
		await draw();
		expect(decoded).toEqual([]);
		await after(1_000);
		expect(drawn.length).toBe(1);

		const made = urls.size;
		await draw("f");
		expect(whatIs(thumbnail().getAttribute("src"))).toBe("a copy");
		// Nothing was made for it: not the capture's URL, not a second copy.
		expect(urls.size).toBe(made);
		expect(decoded.length).toBe(1);
	});

	test("copies are made one to a turn with the window at rest between, and none while the person scrolls", async () => {
		await draw("a", "b", "c");
		const copies = () => thumbnails().filter((img) => whatIs(img.getAttribute("src")) === "a copy").length;
		// A wheel every 100 ms: the window is never at rest, and no capture is decoded under it.
		for (let moved = 0; moved < 10; moved++) {
			await act(async () => document.dispatchEvent(new window.Event("wheel") as unknown as Event));
			await after(100);
		}
		expect(decoded).toEqual([]);

		// At rest, 300 ms after the last wheel (100 of them are gone): the first, and the end of that turn starts the wait again.
		await after(250);
		expect(copies()).toBe(1);
		await after(200);
		expect(copies()).toBe(1);
		await after(1_000);
		expect(copies()).toBe(3);
		expect(decoded.length).toBe(3);
	});

	test("off macOS no copy is made: the engine's own thumbnail of the capture is the better picture there, and it is drawn as it was", async () => {
		os = "linux";
		await draw("f");
		const first = thumbnail().getAttribute("src");
		await after(2_000);
		const img = thumbnail();
		expect(img.getAttribute("src")).toBe(first);
		expect(whatIs(first)).toBe("the capture");
		expect(img.getAttribute("decoding")).toBe("async");
		expect(img.style.aspectRatio).toBe("");
		expect(decoded).toEqual([]);
		expect(revoked).toEqual([]);
	});

	test("a capture no wider than its thumbnail, and one the engine cannot decode, are drawn as they came and not tried again", async () => {
		screen = { width: 96, height: 54 };
		await draw("small");
		await after(1_000);
		expect(whatIs(thumbnail().getAttribute("src"))).toBe("the capture");
		expect(drawn).toEqual([]);

		screen = null;
		await draw("small", "broken");
		await after(1_000);
		expect(thumbnails().map((img) => whatIs(img.getAttribute("src")))).toEqual(["the capture", "the capture"]);
		// One try each.
		expect(decoded.length).toBe(2);
		expect(revoked).toEqual([]);
	});
});

describe("the whole capture is drawn only where it is asked for", () => {
	test("in the thumbnail from a press until the next press elsewhere, so a drag or the system's Copy Image takes the capture; and in the viewer", async () => {
		await draw("f");
		await after(1_000);
		const copy = thumbnail().getAttribute("src");
		expect(whatIs(copy)).toBe("a copy");
		const before = urls.size;

		// Pressed: the thumbnail holds the capture, in the same element and the same box.
		const button = container.querySelector(".picture-open")!;
		const img = thumbnail();
		await press(button);
		expect(thumbnail()).toBe(img);
		const whole = img.getAttribute("src");
		expect(whatIs(whole)).toBe("the capture");
		expect(img.style.aspectRatio).toBe("1568 / 882");
		expect(urls.size).toBe(before + 1);

		// The click that follows opens the viewer on the same picture, already asked for.
		await act(async () => (button as unknown as HTMLElement).click());
		const viewed = document.querySelector(".viewer img");
		expect(viewed?.getAttribute("src")).toBe(whole);
		expect(revoked).not.toContain(whole!);

		// A press in the viewer is a press elsewhere, and the viewer still has its picture.
		await press(viewed!);
		expect(thumbnail().getAttribute("src")).toBe(copy);
		expect(document.querySelector(".viewer img")?.getAttribute("src")).toBe(whole);
		expect(revoked).not.toContain(whole!);

		// Closed: nothing holds the capture.
		await act(async () => window.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape" }) as unknown as Event));
		expect(document.querySelector(".viewer")).toBeNull();
		expect(revoked).toContain(whole!);
		expect(thumbnail().getAttribute("src")).toBe(copy);
	});

	test("opened from the keyboard, the viewer has the capture and the thumbnail keeps its copy", async () => {
		await draw("f");
		await after(1_000);
		const copy = thumbnail().getAttribute("src");
		await act(async () => (container.querySelector(".picture-open") as unknown as HTMLElement).click());
		expect(whatIs(document.querySelector(".viewer img")?.getAttribute("src") ?? null)).toBe("the capture");
		expect(thumbnail().getAttribute("src")).toBe(copy);
	});
});
