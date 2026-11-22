import { afterEach, beforeEach, expect, mock, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { InboxItem, InboxView, ScheduledJob, TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

// task-469: what a colleague's message prompted says so in one quiet line.
// A card or a job a teammate made in a quiet turn one inbox item woke
// carries that item (`fromItem`), and the desk draws "From Huck's message ·
// title" with it, which opens the item. A file or a reply carries it too,
// and the desk draws nothing over it (George, 2026-11-21: to the person,
// inbox items are not mail).

const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });
const real = await import("../src/voice/transcription");
mock.module("../src/voice/transcription", () => ({ ...real, hearsOnThisMac: async () => true }));
const { wire } = await import("../src/wire");

const NOW = Date.now();
const people = new Map([
	["juno", { name: "Juno" }],
	["huck", { name: "Huck" }],
]);

function item(id: string, title: string): InboxItem {
	return {
		id,
		ts: NOW - 60_000,
		from: "huck",
		fromThread: { kind: "dm", key: "huck" },
		to: "juno",
		title,
		body: "",
		wants: "work",
		state: "closed",
		outcome: "done",
		receipts: [{ state: "new", at: NOW - 60_000 }],
		senderTold: 1,
	};
}

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

let act: typeof import("react").act;
let root: Root | null;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

beforeEach(async () => {
	({ teardown } = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} }));
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	const view: InboxView = { personaId: "juno", open: [], closed: [item("w", "Rerun the Linux build")], sent: [], unread: 0 };
	spyOn(wire, "subscribe").mockImplementation(((target: { inbox?: string }, handlers: { snapshot(items: unknown): void }) => {
		if (target.inbox === "juno") handlers.snapshot(view);
		return () => {};
	}) as typeof wire.subscribe);
});

afterEach(async () => {
	await act(async () => root?.unmount());
	root = null;
	await teardown();
});

const press = async (target: Element) => act(async () => void target.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
const lines = () => [...container.querySelectorAll<HTMLButtonElement>(".link-row[data-kind=inbox-from]")];

test("a card a quiet turn one item woke has the line over it, a file from the same turn and a card the person's own turn raised have none", async () => {
	const { Transcript } = await import("../src/components/Transcript");
	const opened: string[] = [];
	const events: TranscriptEvent[] = [
		{
			kind: "agent",
			id: "f1",
			ts: NOW - 50_000,
			text: "",
			fromItem: "w",
			attachments: [{ kind: "file", name: "build.log", path: "/tmp/build.log", size: 12 }],
		},
		{ kind: "human_action", id: "human:a1", ts: NOW - 40_000, actionId: "a1", reason: "Sign the release", status: "pending", delivers: true, fromItem: "w" },
		{ kind: "turn", id: "e1", ts: NOW - 39_000, stopReason: "end_turn", quiet: true },
		{ kind: "user", id: "u1", ts: NOW - 30_000, text: "Ask me before you push" },
		{ kind: "human_action", id: "human:a2", ts: NOW - 20_000, actionId: "a2", reason: "Push the tag?", status: "pending", delivers: true },
	];
	await act(async () =>
		root!.render(<Transcript personaId="juno" name="Juno" people={people} events={events} streaming={[]} live={false} focus={null} onOpenInboxItem={(id) => opened.push(id)} />),
	);
	expect(lines().map((line) => line.querySelector(".link-row-words")?.textContent)).toEqual(["From Huck's message ·Rerun the Linux build"]);
	expect(lines()[0]?.closest("[data-kind=prompted]")?.textContent).toContain("Sign the release");
	expect(container.textContent).toContain("build.log");
	expect(container.querySelector(".said-group .link-row[data-kind=inbox-from]")).toBeNull();
	expect(container.textContent).toContain("Push the tag?");
	await press(lines()[0]!);
	expect(opened).toEqual(["w"]);
});

test("the card in the composer has the line over it too", async () => {
	const { AskCard } = await import("../src/components/transcript/AskCard");
	const { InboxRows } = await import("../src/components/transcript/InboxLines");
	const ask: Extract<TranscriptEvent, { kind: "human_action" }> = {
		kind: "human_action",
		id: "human:a1",
		ts: NOW,
		actionId: "a1",
		reason: "Sign the release",
		status: "pending",
		delivers: true,
		fromItem: "w",
	};
	await act(async () =>
		root!.render(
			<InboxRows.Provider value={{ items: new Map([["w", item("w", "Rerun the Linux build")]]), work: new Map() }}>
				<AskCard ask={ask} personaId="juno" thread={{ kind: "dm", key: "juno" }} ownerName="Juno" people={people} />
			</InboxRows.Provider>,
		),
	);
	expect(container.querySelector(".link-row[data-kind=inbox-from]")?.textContent).toContain("From Huck's message ·Rerun the Linux build");
});

test("a job a quiet turn made says whose message it came from under its row, which opens it", async () => {
	const { Schedules } = await import("../src/components/Schedules");
	const opened: string[] = [];
	const job: ScheduledJob = {
		id: "j1",
		personaId: "juno",
		kind: "schedule",
		prompt: "Check the build again",
		operatorCreated: false,
		nextAt: NOW + 3_600_000,
		createdAt: NOW - 1_000,
		fromItem: "w",
	};
	const gone: ScheduledJob = { ...job, id: "j2", prompt: "Look at the cache", fromItem: "tidied" };
	await act(async () =>
		root!.render(<Schedules personaId="juno" jobs={[job, gone]} backgroundWork focus={false} people={people} onOpenInboxItem={(id) => opened.push(id)} />),
	);
	const source = [...container.querySelectorAll<HTMLButtonElement>("button.group-row-detail")];
	expect(source.map((line) => line.textContent)).toEqual(["From Huck's message · Rerun the Linux build"]);
	expect(container.textContent).toContain("From a colleague’s message");
	await press(source[0]!);
	expect(opened).toEqual(["w"]);
});
