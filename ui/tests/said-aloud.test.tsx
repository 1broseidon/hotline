import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import type { TranscriptEvent } from "../src/generated/contract";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Transcript, SaidAloud } = await import("../src/components/Transcript");

const reply: Extract<TranscriptEvent, { kind: "agent" }> = {
	kind: "agent",
	id: "a1",
	ts: 1,
	text: "The build fails for two reasons:\n\n1. **Missing API key.**\n2. **Port 8080 in use.**",
	spoken: "Two things are wrong: the API key is missing and the port's already in use. I've put both fixes in the chat.",
};

function transcript(events: TranscriptEvent[]): string {
	return renderToStaticMarkup(<Transcript personaId="ada" name="Ada" events={events} streaming={[]} live={false} focus={null} />);
}

describe("a reply said on a call", () => {
	test("is the written version, with what was said as a transcript line above it", () => {
		const html = transcript([reply]);
		const line = html.indexOf("said-aloud");
		const bubble = html.indexOf("speech said-them");
		expect(line).toBeGreaterThan(-1);
		expect(line).toBeLessThan(bubble);
		expect(html).toContain("Said on the call: ");
		expect(html).toContain("the port&#x27;s already in use");
		expect(html).toContain("Missing API key.");
		expect(html).toContain('aria-expanded="false"');
	});

	test("a reply that was not said has no transcript line", () => {
		const { spoken: _spoken, ...typed } = reply;
		expect(transcript([typed])).not.toContain("said-aloud");
		expect(transcript([{ ...reply, spoken: " " }])).not.toContain("said-aloud");
	});
});

describe("the transcript line", () => {
	let act: typeof import("react").act;
	let root: Root;
	let container: HTMLDivElement;
	let dom: Window;
	let restores: (() => void)[];

	beforeEach(async () => {
		dom = new Window();
		restores = [];
		for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
			const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
			Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
			restores.push(() => (descriptor ? Object.defineProperty(globalThis, key, descriptor) : Reflect.deleteProperty(globalThis, key)));
		}
		act = (await import("react")).act;
		const { createRoot } = await import("react-dom/client");
		container = document.createElement("div");
		document.body.append(container);
		root = createRoot(container);
	});

	afterEach(async () => {
		await act(async () => root.unmount());
		for (const restore of restores.reverse()) restore();
		await dom.happyDOM.close();
	});

	test("is one line until pressed, then the whole of what was said, and folds again", async () => {
		await act(async () => root.render(<SaidAloud text={reply.spoken ?? ""} />));
		const line = container.querySelector("button.said-aloud") as unknown as HTMLButtonElement;
		expect(line.getAttribute("aria-expanded")).toBe("false");
		expect(line.classList.contains("said-aloud-open")).toBe(false);
		expect(line.textContent).toBe(`Said on the call: ${reply.spoken}`);
		await act(async () => line.click());
		expect(line.getAttribute("aria-expanded")).toBe("true");
		expect(line.classList.contains("said-aloud-open")).toBe(true);
		await act(async () => line.click());
		expect(line.getAttribute("aria-expanded")).toBe("false");
	});
});
