import { afterEach, beforeEach, expect, mock, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { InboxItem, InboxView, ScheduledJob, TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

// George, 2026-11-21: to the person, inbox items are not mail; they see work
// (threads) and problems. The desk counts nothing unread, heads no Path
// section "Inbox", and draws no "From Huck's message" over a reply or a
// file. What stays is what asks the person something or says what a
// teammate did: the line over a card and under a job, the card a held send
// waits on and "Awaiting approval" on its sender, the card after twelve
// messages, the could-not-wake line, the folded line under a reply and the
// Path's rows under Teammates. Each is held here.

const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });
const real = await import("../src/voice/transcription");
mock.module("../src/voice/transcription", () => ({ ...real, hearsOnThisMac: async () => true }));
const { wire } = await import("../src/wire");

const NOW = Date.now();
/** Long enough ago that the chat's pacing shows every bubble at once. */
const THEN = NOW - 300_000;
const people = new Map([
	["juno", { name: "Juno" }],
	["huck", { name: "Huck" }],
	["ivy", { name: "Ivy" }],
]);

function item(id: string, more: Partial<InboxItem> = {}): InboxItem {
	return {
		id,
		ts: THEN,
		from: "huck",
		fromThread: { kind: "dm", key: "huck" },
		to: "juno",
		title: "Rerun the Linux build",
		body: "",
		wants: "work",
		state: "closed",
		outcome: "done",
		receipts: [{ state: "new", at: THEN }],
		senderTold: 1,
		...more,
	};
}

/** Juno's inbox: what Huck asked of it, and what it sent Ivy, held for the person. */
const { outcome: _done, ...open } = item("held", { from: "juno", to: "ivy", title: "Rotate the staging keys", held: true, state: "new", receipts: [] });
const held: InboxItem = open;
const view: InboxView = { personaId: "juno", open: [], closed: [item("w")], sent: [held], unread: 4 };

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

const tool = (id: string, title: string): TranscriptEvent => ({ kind: "tool", id, ts: THEN, toolCallId: id, title, status: "completed" });
const press = async (target: Element) => act(async () => void target.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));
const fromLines = () => [...container.querySelectorAll(".link-row[data-kind=inbox-from]")].map((line) => line.querySelector(".link-row-words")?.textContent);

async function drawTranscript(events: TranscriptEvent[]) {
	const { Transcript } = await import("../src/components/Transcript");
	await act(async () => root!.render(<Transcript personaId="juno" name="Juno" people={people} events={events} streaming={[]} live={false} focus={null} onOpenInboxItem={() => {}} />));
}

test("a reply and a file a colleague's message woke have no From line; the permission card the same turn raised keeps it", async () => {
	await drawTranscript([
		{ kind: "agent", id: "p1", ts: THEN, text: "The Linux build is green again.", fromItem: "w" },
		{ kind: "agent", id: "f1", ts: THEN, text: "", fromItem: "w", attachments: [{ kind: "file", name: "build.log", path: "/tmp/build.log", size: 12 }] },
		{ kind: "permission", id: "perm:r1", ts: THEN, requestId: "r1", title: "git push origin main", options: [{ optionId: "allow", name: "Allow" }, { optionId: "deny", name: "Deny" }], fromItem: "w" },
		{ kind: "turn", id: "e1", ts: THEN, stopReason: "end_turn", quiet: true },
	]);
	expect(container.textContent).toContain("The Linux build is green again.");
	expect(container.textContent).toContain("build.log");
	expect(container.querySelector(".said-group .link-row[data-kind=inbox-from]")).toBeNull();
	expect(fromLines()).toEqual(["From Huck's message ·Rerun the Linux build"]);
	expect(container.querySelector(".link-row[data-kind=inbox-from]")?.closest("[data-kind=prompted]")?.textContent).toContain("git push origin main");
});

test("a held send's approval card stays, with the From line of the message that woke it", async () => {
	await drawTranscript([
		{
			kind: "permission",
			id: "perm:collab:held",
			ts: THEN,
			requestId: "collab:held",
			title: "Juno wants Ivy to do work\n\nIvy has more access than Juno. If you allow this, Ivy does the work with its own access.",
			options: [
				{ optionId: "allow", name: "Allow" },
				{ optionId: "always", name: "Always" },
				{ optionId: "deny", name: "Deny" },
			],
			fromItem: "w",
		},
	]);
	const card = container.querySelector(".ask-card[data-kind=collaboration]");
	expect(card?.textContent).toContain("Juno wants Ivy to do work");
	expect([...(card?.querySelectorAll("button") ?? [])].map((button) => button.textContent)).toEqual(["Allow", "Always", "Deny"]);
	expect(fromLines()).toEqual(["From Huck's message ·Rerun the Linux build"]);
});

test("the sender's Path lists what it sent and sorted under Teammates, not Inbox, each with its detail, and a held send says Awaiting approval", async () => {
	const { TurnInbox } = await import("../src/components/InboxPane");
	const opened: string[] = [];
	await act(async () =>
		root!.render(
			<TurnInbox
				personaId="juno"
				rows={[
					{ kind: "sorted", key: "s1:0", entry: { item: "w", title: "Rerun the Linux build", from: "huck", state: "closed", outcome: "done" } },
					{ kind: "sent", key: "inbox-sent:held", sent: { kind: "inbox_sent", id: "inbox-sent:held", ts: THEN, item: "held", to: "ivy", title: "Rotate the staging keys" } },
				]}
				people={people}
				onOpenItem={(id) => opened.push(id)}
			/>,
		),
	);
	const section = container.querySelector("section[aria-label=Teammates]");
	expect(section?.querySelector(".spine-rule-title")?.textContent).toBe("Teammates");
	expect(container.textContent).not.toContain("Inbox");
	const rows = [...container.querySelectorAll<HTMLButtonElement>(".list-row")];
	expect(rows.map((row) => [row.querySelector(".list-title-text")?.textContent, row.querySelector(".list-line")?.textContent])).toEqual([
		["Rerun the Linux build", "From Huck · Done"],
		["Rotate the staging keys", "To Ivy · Awaiting approval"],
	]);
	await press(rows[1]!);
	expect(opened).toEqual(["held"]);
});

test("the card after twelve messages without the person stays", async () => {
	await drawTranscript([{ kind: "exchange_paused", id: "pause-1", ts: THEN, withPersonaId: "huck", withName: "Huck", exchanges: 12, status: "pending" }]);
	expect(container.textContent).toContain("Juno and Huck paused after 12 messages without you.");
	expect([...container.querySelectorAll(".card-actions button")].map((button) => button.textContent)).toEqual(["Keep going", "Stop"]);
});

test("the line saying Hotline could not wake a teammate stays in the chat", async () => {
	await drawTranscript([
		{ kind: "agent", id: "wake-failure", ts: THEN, text: "Hotline: I couldn't wake Huck for your request: the agent is out of quota. The inbox item is still waiting." },
	]);
	expect(container.textContent).toContain("Hotline: I couldn't wake Huck for your request: the agent is out of quota.");
});

test("the folded line under a reply still says what the turn read and sent", async () => {
	await drawTranscript([
		{ kind: "user", id: "u1", ts: THEN, text: "Ask Ivy to rotate the staging keys." },
		tool("t1", "read_inbox"),
		{
			kind: "inbox_sorted",
			id: "s1",
			ts: THEN,
			open: 0,
			unread: 0,
			entries: [
				{ item: "w", title: "Rerun the Linux build", from: "huck", state: "closed", outcome: "done" },
				{ item: "w2", title: "Changelog wording", from: "huck", state: "closed", outcome: "noted" },
			],
		},
		{ kind: "inbox_sent", id: "inbox-sent:held", ts: THEN, item: "held", to: "ivy", title: "Rotate the staging keys" },
		{ kind: "agent", id: "a1", ts: THEN, text: "Asked Ivy." },
		{ kind: "turn", id: "e1", ts: THEN, stopReason: "end_turn" },
	]);
	expect([...container.querySelectorAll(".steps-caption [data-kind=inbox-words]")].map((line) => line.textContent)).toEqual(["Read 2 messages, sent a message"]);
	expect(container.textContent).not.toContain("unread");
});

test("a job a colleague's message made keeps the From line under its row", async () => {
	const { Schedules } = await import("../src/components/Schedules");
	const job: ScheduledJob = { id: "j1", personaId: "juno", kind: "schedule", prompt: "Check the build again", operatorCreated: false, nextAt: NOW + 3_600_000, createdAt: NOW - 1_000, fromItem: "w" };
	await act(async () => root!.render(<Schedules personaId="juno" jobs={[job]} backgroundWork focus={false} people={people} onOpenInboxItem={() => {}} />));
	expect([...container.querySelectorAll("button.group-row-detail")].map((line) => line.textContent)).toEqual(["From Huck's message · Rerun the Linux build"]);
});
