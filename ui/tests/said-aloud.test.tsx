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

	test("a combined note opens every source call", async () => {
		const opened: unknown[] = [];
		await act(async () => root.render(<AgentSay personaId="ada" event={{ kind: "agent", id: "summary", ts: 2, text: "Beacon repair agreed.", from: { kind: "call", thread: "second", sources: [{ kind: "call", key: "first" }, { kind: "call", key: "second" }] } }} run={{ top: true, bottom: true }} reactions={undefined} onOpenThread={(thread) => opened.push(thread)} />));
		for (const label of ["Open call 1", "Open call 2"]) {
			const button = Array.from(container.querySelectorAll("button")).find((button) => button.textContent === label) as HTMLButtonElement;
			expect(button).toBeDefined();
			await act(async () => button.click());
		}
		expect(opened).toEqual([{ thread: { kind: "call", key: "first" } }, { thread: { kind: "call", key: "second" } }]);
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


describe("a call thread is a transcript", () => {
    test("names both speakers and shows every tool inline in order", () => {
        const events: TranscriptEvent[] = [
            { kind: "user", id: "u", ts: 1, text: "Check the mascot." },
            { kind: "agent", id: "a", ts: 2, text: "The written plan.", spoken: "I will check Bramble." },
            { kind: "tool", id: "t1", ts: 3, toolCallId: "tc1", title: "Read mascot.txt", toolKind: "read_file", status: "completed", output: [{ type: "text", text: "Bramble, a wiry terrier.\nMore detail." }] },
            { kind: "tool", id: "t2", ts: 4, toolCallId: "tc2", title: "Save the plan", toolKind: "write_file", status: "completed", output: [{ type: "text", text: "Saved mascot.txt." }] },
            { kind: "agent", id: "a2", ts: 5, text: "Ready to implement." },
        ];
        const html = renderToStaticMarkup(<Transcript personaId="ada" name="Ada" thread={{ kind: "call", key: "c1" }} events={events} streaming={[]} live={false} focus={null} />);
        expect(html).toContain("You");
        expect(html).toContain("Ada");
        expect(html).toContain("I will check Bramble.");
        expect(html).toContain("Bramble, a wiry terrier. More detail.");
        expect(html).not.toContain(">2 steps<");
        expect(html).not.toContain("steps-caption");
        expect(html.indexOf("I will check Bramble.")).toBeLessThan(html.indexOf("read_file"));
        expect(html.indexOf("read_file")).toBeLessThan(html.indexOf("write_file"));
        expect(html.indexOf("write_file")).toBeLessThan(html.indexOf("Ready to implement."));
        const work = renderToStaticMarkup(<Transcript personaId="ada" name="Ada" thread={{ kind: "side", key: "s1" }} events={events} streaming={[]} live={false} focus={null} />);
        expect(work).toContain("2 steps");
    });
});

describe("a reply that was said as it was written", () => {
    // A spoken-only reply on a call: the first bubble keeps the whole of what
    // was heard and the later bubbles hold the rest of the same words.
    const said = "Quite a bit, mostly QA.\n\nMidweek I audited the board.\n\nThen you ran backend switching.";
    const events: TranscriptEvent[] = [
        { kind: "user", id: "u", ts: 1, text: "What did we talk about this week?" },
        { kind: "agent", id: "r", ts: 2, text: "Quite a bit, mostly QA.", spoken: said },
        { kind: "agent", id: "r-2", ts: 2, text: "Midweek I audited the board." },
        { kind: "agent", id: "r-3", ts: 2, text: "Then you ran backend switching." },
    ];
    const count = (html: string, text: string) => html.split(text).length - 1;

    test("a call's transcript says each sentence once, with no written reply to open", () => {
        const html = renderToStaticMarkup(<Transcript personaId="ada" name="Ada" thread={{ kind: "call", key: "c1" }} events={events} streaming={[]} live={false} focus={null} />);
        for (const line of ["Quite a bit, mostly QA.", "Midweek I audited the board.", "Then you ran backend switching."]) expect(count(html, line)).toBe(1);
        expect(html).not.toContain("Written reply");
    });

    test("the chat draws no said-aloud line over the same words", () => {
        const html = transcript(events);
        expect(html).not.toContain("said-aloud");
        expect(count(html, "Midweek I audited the board.")).toBe(1);
    });

    test("a reply whose versions differ is said once, with its whole written version under it", () => {
        const differs: TranscriptEvent[] = [
            { kind: "agent", id: "d", ts: 2, text: "The build fails for two reasons:", spoken: "Two things are wrong, both fixes are in the chat." },
            { kind: "agent", id: "d-2", ts: 2, text: "1. **Missing API key.**\n2. **Port 8080 in use.**" },
        ];
        const html = renderToStaticMarkup(<Transcript personaId="ada" name="Ada" thread={{ kind: "call", key: "c1" }} events={differs} streaming={[]} live={false} focus={null} />);
        expect(count(html, "Two things are wrong")).toBe(1);
        expect(count(html, "Missing API key.")).toBe(1);
        expect(html.indexOf("Written reply")).toBeLessThan(html.indexOf("Missing API key."));
        expect(transcript(differs)).toContain("said-aloud");
    });
});
