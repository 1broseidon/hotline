import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { LinkEvent } from "../src/dock";
import { installDom } from "./dom";

let act: typeof import("react").act;
let Transcript: typeof import("../src/components/Transcript").Transcript;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

beforeEach(async () => {
	const installed = installDom({ requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	Object.assign(globalThis, { ResizeObserver: installed.dom.ResizeObserver });
	act = (await import("react")).act;
	({ Transcript } = await import("../src/components/Transcript"));
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

test("a call row follows its own turn and then its parked link, and still opens the call", async () => {
	let link: LinkEvent = { kind: "link", id: "link:call:c1", ts: 1, thread: "c1", threadKind: "call", title: "Release checks", state: "live" };
	const opened: string[] = [];
	const draw = async (workingCalls: string[]) => {
		await act(async () => root.render(
			<Transcript personaId="ada" name="Ada" events={[link]} streaming={[]} live={false} focus={null}
				workingCalls={workingCalls} onOpenThread={(ref) => opened.push(ref.thread.key)} />,
		));
		return container.querySelector("button.rule-line")!;
	};
	expect((await draw(["c1"])).textContent).toBe("Release checks · running· Open");
	expect((await draw(["other-call"])).textContent).toBe("Release checks · open· Open");
	link = { ...link, state: "parked" };
	const parked = await draw([]);
	expect(parked.textContent).toBe("Release checks · parked· Open");
	expect(parked.getAttribute("aria-label")).toBe("Release checks · parked. Open it");
	await act(async () => (parked as HTMLButtonElement).click());
	expect(opened).toEqual(["c1"]);
	// A resumed turn can arrive before the rewritten link.
	expect((await draw(["c1"])).textContent).toBe("Release checks · running· Open");
});

test("a call thread folds speech updates and opens work results as work", async () => {
	const { NOTHING_YET, reduceThread } = await import("../src/tape");
	const said = (text: string): import("../src/generated/contract").TranscriptEvent => ({ kind: "agent", id: "front-c1-0", ts: 1, text });
	let held = reduceThread(NOTHING_YET, { type: "snapshot", items: [said("Done."), said("Done. Saved in crane.txt.")] });
	held = reduceThread(held, { type: "event", item: said("Done. Saved in crane.txt. It’s in the chat.") });
	held = reduceThread(held, { type: "event", item: { kind: "tool", id: "work-result", ts: 2, toolCallId: "work-result", title: "Work result", status: "completed", output: [{ type: "text", text: "Farmer Crane tends the field.\nThe corn bows in the wind.\nEvening brings him home." }] } });
	await act(async () => root.render(<Transcript personaId="ada" name="Ada" events={held.events} streaming={held.streaming} live={false} focus={null} />));
	expect(container.textContent?.match(/Done\./g)).toHaveLength(1);
	expect(container.textContent).toContain("It’s in the chat.");
	// Work is disclosed by the same working control as a tool's output.
	const steps = container.querySelector<HTMLButtonElement>(".steps-caption");
	if (steps) await act(async () => steps.click());
	const result = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Work result");
	expect(result).toBeDefined();
	await act(async () => result!.click());
	expect(container.querySelector(".step-out")?.textContent).toContain("Farmer Crane tends the field.");
	expect(container.textContent?.match(/Farmer Crane/g)).toHaveLength(1);
});


test("call cards link home and home cards name and open their call", async () => {
    const from = { kind: "call" as const, thread: "c1", title: "Release checks" };
    const card: import("../src/generated/contract").TranscriptEvent = {
        kind: "human_action", id: "human:q1", ts: 1, actionId: "q1", reason: "Use blue?",
        question: true, status: "pending", from,
    };
    const opened: import("../src/components/Transcript").ThreadRef[] = [];
    const draw = async (kind: "dm" | "call", settled = false) => {
        await act(async () => root.render(<Transcript personaId="ada" name="Ada"
            thread={{ kind, key: kind === "dm" ? "ada" : "c1" }}
            events={[{ ...card, status: settled ? "done" : "pending" }]} streaming={[]} live={false} focus={null}
            onOpenThread={(ref) => opened.push(ref)} />));
    };
    await draw("dm");
    const origin = [...container.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.includes("From the call"))!;
    expect(origin.textContent).toContain("Release checks");
    await act(async () => origin.click());
    expect(opened.pop()).toEqual({ thread: { kind: "call", key: "c1" }, title: "Release checks" });
    expect(container.textContent).toContain("Yes");
    await draw("call");
    expect(container.textContent).not.toContain("From the call");
    const link = [...container.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.includes("Open card in home chat"))!;
    expect(link.textContent).toContain("Needs you: Use blue?");
    await act(async () => link.click());
    expect(opened.pop()).toEqual({ thread: { kind: "dm", key: "ada" }, eventId: "human:q1" });
    await draw("call", true);
    expect(container.textContent).toContain("Settled: Use blue?");
    await draw("dm", true);
    expect(container.textContent).toContain("Needed you · Yes");
});

test("a six-file call result is one compact message with every file's actions and metadata", async () => {
	const event: import("../src/generated/contract").TranscriptEvent = {
		kind: "agent", id: "files", ts: 1, text: "6 files from the call", from: { kind: "call", thread: "c1" },
		attachments: Array.from({ length: 6 }, (_, i) => ({ kind: "file", name: `note-${i}.txt`, path: `/files/${i}`, created: i % 2 === 0, size: 8 })),
	};
	await act(async () => root.render(<Transcript personaId="ada" name="Ada" events={[event]} streaming={[]} live={false} focus={null} />));
	expect(container.querySelectorAll(".said-group")).toHaveLength(1);
	expect(container.textContent).toContain("From the call");
	const files = [...container.querySelectorAll(".sent-card")];
	expect(files).toHaveLength(6);
	for (const [i, file] of files.entries()) {
		expect(file.textContent).toContain(`note-${i}.txt`);
		expect(file.textContent).toContain(i % 2 === 0 ? "Created" : "Edited");
		expect(file.textContent).toContain("8 B");
		expect([...file.querySelectorAll("button")].map(button => button.textContent)).toContain("Open");
		expect([...file.querySelectorAll("button")].map(button => button.textContent)).toContain("Save…");
	}
});


test("every call file opens its own stored bytes as text without executing markup", async () => {
	const { wire } = await import("../src/wire");
	const reads: number[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (name: string, params: { index: number }) => {
		expect(name).toBe("file.read");
		reads.push(params.index);
		return { name: "note.txt", mimeType: "text/plain", data: btoa(`<script>file ${params.index}</script>`) };
	}) as typeof wire.command);
	try {
		const event: import("../src/generated/contract").TranscriptEvent = { kind: "agent", id: "open-files", ts: 1, text: "6 files", from: { kind: "call", thread: "c1" }, attachments: Array.from({ length: 6 }, (_, i) => ({ kind: "file", name: `note-${i}.txt`, path: `/files/${i}`, created: true, size: 8 })) };
		await act(async () => root.render(<Transcript personaId="ada" name="Ada" events={[event]} streaming={[]} live={false} focus={null} />));
		for (let index = 0; index < 6; index++) {
			const card = container.querySelectorAll(".sent-card")[index]!;
			const open = [...card.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Open")!;
			await act(async () => open.click());
			const dialog = document.querySelector('[role="dialog"]')!;
			expect(dialog.textContent).toContain(`<script>file ${index}</script>`);
			expect(dialog.querySelector("script")).toBeNull();
			const close = [...dialog.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Close")!;
			await act(async () => close.click());
		}
		expect(reads).toEqual([0, 1, 2, 3, 4, 5]);
	} finally { command.mockRestore(); }
});

test("a stored-file preview pages forward and back and shows binary bytes without opening them", async () => {
	const { wire } = await import("../src/wire");
	const { SentPreview } = await import("../src/components/SentPreview");
	const offsets: number[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (_name: string, params: { offset: number }) => {
		offsets.push(params.offset);
		return params.offset === 0 ? { data: btoa("Bramble"), next: 7 } : { data: btoa("\0\x01\x02") };
	}) as typeof wire.command);
	try {
		await act(async () => root.render(<SentPreview sent={{ personaId: "ada", eventId: "paged", index: 0, file: { kind: "file", name: "data.bin", path: "/files/data.bin" } }} actions={null} onClose={() => {}} />));
		const press = async (label: string) => {
			const button = [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find(item => item.textContent === label)!;
			await act(async () => button.click());
		};
		expect(document.querySelector('[role="dialog"]')?.textContent).toContain("Bramble");
		await press("Next");
		expect(document.querySelector('[role="dialog"]')?.textContent).toContain("Binary preview");
		expect(document.querySelector('[role="dialog"] pre')?.textContent).toBe("00000007  00 01 02");
		await press("Previous");
		expect(document.querySelector('[role="dialog"] pre')?.textContent).toBe("Bramble");
		expect(offsets).toEqual([0, 7, 0]);
	} finally { command.mockRestore(); }
});
