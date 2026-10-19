import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

// A long conversation: older lines paged in above the one being read, a
// diagram far down the tape, and captures of the computer's screen.

const ROW = 100;
const VIEW = 500;

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let Transcript: typeof import("../src/components/Transcript").Transcript;
let frames: FrameRequestCallback[];
let observed: { target: Element; root: Element | null; callback: IntersectionObserverCallback }[];
let revoked: string[];
const revoke = URL.revokeObjectURL;

/** A stand-in observer the test drives: it reports what it was told to watch and fires on demand. */
class Observer {
	constructor(
		private callback: IntersectionObserverCallback,
		private options: IntersectionObserverInit = {},
	) {}
	observe(target: Element) {
		observed.push({ target, root: (this.options.root as Element | null) ?? null, callback: this.callback });
	}
	disconnect() {
		observed = observed.filter((one) => one.callback !== this.callback);
	}
	unobserve() {}
	takeRecords() {
		return [];
	}
}

beforeEach(async () => {
	frames = [];
	observed = [];
	revoked = [];
	URL.revokeObjectURL = (url: string) => {
		revoked.push(url);
		revoke(url);
	};
	const installed = installDom({
		requestAnimationFrame: (callback: FrameRequestCallback) => frames.push(callback),
		cancelAnimationFrame: () => {},
		IntersectionObserver: Observer,
	});
	teardown = installed.teardown;
	Object.assign(globalThis, {
		ResizeObserver: installed.dom.ResizeObserver,
		MutationObserver: installed.dom.MutationObserver,
		CSS: installed.dom.CSS,
	});
	act = (await import("react")).act;
	({ Transcript } = await import("../src/components/Transcript"));
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	URL.revokeObjectURL = revoke;
	await teardown();
});

function lines(from: number, to: number): TranscriptEvent[] {
	const made: TranscriptEvent[] = [];
	for (let n = from; n <= to; n++) made.push({ kind: n % 2 ? "agent" : "user", id: `e${n}`, ts: n, text: `Line ${n}` } as TranscriptEvent);
	return made;
}

/** The scroller, laid out as a browser would: each row a fixed height, the view a fixed window on them. */
function laidOut(): HTMLElement {
	const scroller = container.querySelector(".scroller") as unknown as HTMLElement;
	Object.defineProperty(scroller, "scrollHeight", { configurable: true, get: () => container.querySelectorAll(".tape-row").length * ROW });
	Object.defineProperty(scroller, "clientHeight", { configurable: true, get: () => VIEW });
	return scroller;
}

/** A scroll, and the frame the transcript answers it on. */
async function scrolled(scroller: HTMLElement) {
	await act(async () => {
		scroller.dispatchEvent(new window.Event("scroll") as unknown as Event);
	});
	await act(async () => {
		const due = frames;
		frames = [];
		for (const frame of due) frame(0);
	});
}

describe("older lines arriving", () => {
	test("leave the line being read where it was", async () => {
		let events = lines(51, 100);
		const asked: (string | undefined)[] = [];
		const draw = (more: boolean) =>
			root.render(
				<Transcript personaId="ada" name="Ada" events={events} streaming={[]} live={false} focus={null} more={more} onEarlier={earlier} />,
			);
		async function earlier(through?: string) {
			asked.push(through);
			events = lines(1, 100);
			draw(false);
		}
		await act(async () => draw(true));
		const scroller = laidOut();
		// The column was measured before it had a height here; the first
		// frame takes the layout in, as a resize would in the window.
		await scrolled(scroller);
		expect(scroller.scrollTop).toBe(50 * ROW);

		// Scrolled up near the top: reading line 53, two rows down the page.
		scroller.scrollTop = 2 * ROW;
		await scrolled(scroller);

		expect(asked).toEqual([undefined]);
		expect(container.querySelectorAll(".tape-row").length).toBe(100);
		// Fifty rows arrived above, so the same line is fifty rows further down.
		expect(scroller.scrollTop).toBe(52 * ROW);
	});

	test("a search hit above the window pages back to it", async () => {
		const asked: (string | undefined)[] = [];
		await act(async () =>
			root.render(
				<Transcript
					personaId="ada"
					name="Ada"
					events={lines(51, 100)}
					streaming={[]}
					live={false}
					focus={{ eventId: "e7", at: 1 }}
					more
					onEarlier={async (through) => {
						asked.push(through);
					}}
				/>,
			),
		);
		expect(asked).toEqual(["e7"]);
	});
});

describe("a diagram", () => {
	const diagram: TranscriptEvent = { kind: "agent", id: "d1", ts: 1, text: "```mermaid\ngraph TD; A-->B\n```" } as TranscriptEvent;

	test("is not drawn until it comes near the view, and is watched against its scroller", async () => {
		await act(async () => root.render(<Transcript personaId="ada" name="Ada" events={[diagram]} streaming={[]} live={false} focus={null} />));
		const block = container.querySelector(".mermaid-block");
		expect(block).not.toBeNull();
		expect(observed.map((one) => one.target)).toEqual([block as Element]);
		expect(observed[0]?.root).toBe(container.querySelector(".scroller") as Element);

		// Far away, nothing loads mermaid however long it waits.
		await act(async () => new Promise((done) => setTimeout(done, 400)));
		expect(document.head.querySelector("script")).toBeNull();

		// Near: the observer lets go and the drawing starts.
		const { callback } = observed[0]!;
		await act(async () => callback([{ isIntersecting: true, target: block } as unknown as IntersectionObserverEntry], {} as IntersectionObserver));
		expect(observed).toEqual([]);
		// Bun cannot load mermaid's browser build, so here the drawing ends
		// in the block's own note; in the window it ends in the diagram.
		const started = () => document.head.querySelector("script") !== null || container.querySelector(".mermaid-note") !== null;
		for (let waited = 0; waited < 5_000 && !started(); waited += 100) {
			await act(async () => new Promise((done) => setTimeout(done, 100)));
		}
		expect(started()).toBe(true);
	});
});

describe("a capture of the computer's screen", () => {
	const png = "data:image/png;base64,iVBORw0KGgo=";
	const frame = (id: string): TranscriptEvent => ({ kind: "computer_frame", id, ts: 1, dataUrl: png });

	function shown(): string[] {
		return [...container.querySelectorAll("img")].map((img) => img.getAttribute("src") ?? "");
	}

	test("is drawn from one blob URL per event, kept across a switch away and back", async () => {
		const draw = (events: TranscriptEvent[], personaId = "ada") =>
			root.render(<Transcript personaId={personaId} name="Ada" events={events} streaming={[]} live={false} focus={null} />);
		await act(async () => draw([frame("f1"), frame("f2")]));
		const [first, second] = shown();
		expect(first).toStartWith("blob:");
		expect(second).toStartWith("blob:");
		expect(first).not.toBe(second);

		await act(async () => draw(lines(1, 2), "grace"));
		await act(async () => draw([frame("f1"), frame("f2")]));
		expect(shown()).toEqual([first!, second!]);
		expect(revoked).toEqual([]);
	});

	test("lets go of the least recently shown, never one on screen", async () => {
		const many = Array.from({ length: 40 }, (_, n) => frame(`m${n}`));
		await act(async () =>
			root.render(<Transcript personaId="ada" name="Ada" events={many} streaming={[]} live={false} focus={null} />),
		);
		const urls = shown();
		expect(new Set(urls).size).toBe(40);
		// Every one is on screen at once, past what is kept, and none is let go.
		expect(revoked).toEqual([]);

		await act(async () =>
			root.render(<Transcript personaId="grace" name="Grace" events={lines(1, 2)} streaming={[]} live={false} focus={null} />),
		);
		// Off screen, the cache trims to what it keeps, oldest first.
		const ours = revoked.filter((url) => urls.includes(url));
		expect(ours.length).toBeGreaterThan(0);
		expect(ours.length).toBeLessThan(40);
		expect(ours).toEqual(urls.slice(0, ours.length));
	});
});
