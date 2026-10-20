import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import type { TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { AgentSay } = await import("../src/components/transcript/Bubbles");
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

describe("a post from another thread", () => {
	test("names where it came from above it, from the stamp", () => {
		const posted = { kind: "agent", id: "p1", ts: 2, text: "the scheduler fix is in" } as const;
		const fromCall = transcript([{ ...posted, from: { thread: "c1", kind: "call" } }]);
		expect(fromCall).toContain("From the call");
		expect(transcript([{ ...posted, from: { thread: "c1", kind: "call", title: "Call" } }])).toContain("From the call");
		expect(transcript([{ ...posted, from: { thread: "c1", kind: "call", title: "Harbour plan" } }])).toContain("From Harbour plan");
		expect(fromCall.indexOf("From the call")).toBeLessThan(fromCall.indexOf("speech said-them"));
		const scheduled = { jobId: "j1", kind: "loop", name: "Crane check", operatorCreated: true, quiet: true } as const;
		expect(transcript([{ ...posted, scheduled }])).toContain("From a scheduled job");
		expect(transcript([posted])).not.toContain("From ");
	});
});

describe("the transcript line", () => {
	let act: typeof import("react").act;
	let root: Root;
	let container: HTMLDivElement;
	let teardown: () => Promise<void>;

	beforeEach(async () => {
		({ teardown } = installDom());
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

	test("Open the call opens the full source thread of a summary", async () => {
		const opened: unknown[] = [];
		await act(async () => root.render(<AgentSay personaId="ada" event={{ kind: "agent", id: "summary", ts: 2, text: "Eddystone history: four towers guarded the reef.", from: { kind: "call", thread: "eddystone" } }} run={{ top: true, bottom: true }} reactions={undefined} onOpenThread={(thread) => opened.push(thread)} />));
		const button = Array.from(container.querySelectorAll("button")).find((button) => button.textContent === "Open the call") as HTMLButtonElement;
		expect(button).toBeDefined();
		await act(async () => button.click());
		expect(opened).toEqual([{ thread: { kind: "call", key: "eddystone" } }]);
		expect(container.textContent).toContain("From the call");
		expect(container.textContent).not.toContain("From Call");
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
