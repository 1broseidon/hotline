import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { InboxItem, InboxSortedEntry, InboxView, TranscriptEvent } from "../src/generated/contract";
import type { InboxSent } from "../src/inbox";
import type { Block } from "../src/scheduledRuns";
import { installDom } from "./dom";

// task-456: the desk draws the inbox (docs/design/agent-inbox.md, "On the
// desk"). A sender's conversation has one quiet row per send, live from its
// inbox; a receiver's has one card per sort; the composer counts what the
// teammate has not read; the pane lists its inbox and opens one item with
// its receipts. The person reads; nothing here sends or sorts.

const query = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
Object.assign(globalThis, { window: { matchMedia: query } });
const real = await import("../src/voice/transcription");
mock.module("../src/voice/transcription", () => ({ ...real, hearsOnThisMac: async () => true }));
const inbox = await import("../src/inbox");
const { groupInbox } = await import("../src/scheduledRuns");
const { wire } = await import("../src/wire");

const NOW = Date.now();
const people = new Map([
	["chop", { name: "Pork Chop" }],
	["juno", { name: "Juno" }],
	["ivy", { name: "Ivy" }],
	["huck", { name: "Huck" }],
]);
const nameOf = (id: string) => people.get(id)?.name;

/** An item as the desk keeps it, sent `ago` ms before now. */
function item(id: string, more: Partial<InboxItem> = {}, ago = 60_000): InboxItem {
	return {
		id,
		ts: NOW - ago,
		from: "chop",
		fromThread: { kind: "dm", key: "chop" },
		to: "juno",
		title: `Title ${id}`,
		body: `Words of ${id}`,
		wants: "answer",
		state: "new",
		receipts: [{ state: "new", at: NOW - ago }],
		senderTold: 1,
		...more,
	};
}
const sent = (id: string, to: string, title: string, ts = NOW): TranscriptEvent => ({ kind: "inbox_sent", id: `inbox-sent:${id}`, ts, item: id, to, title });
const said = (id: string, text: string): TranscriptEvent => ({ kind: "agent", id, ts: NOW, text });
const ended = (id: string): TranscriptEvent => ({ kind: "turn", id, ts: NOW, stopReason: "end_turn" });
const blocks = (events: TranscriptEvent[]): Block[] => events.map((event) => ({ kind: "event", event }) as Block);
const view = (more: Partial<InboxView> = {}): InboxView => ({ personaId: "juno", open: [], closed: [], sent: [], unread: 0, ...more });

describe("task-456 the sender's rows", () => {
	test("items one send made to several teammates under one title are one row; another title, or anything between, is another", () => {
		const grouped = groupInbox(
			blocks([
				said("a1", "Sending both now."),
				sent("i1", "juno", "Release moves to Thursday"),
				sent("i2", "ivy", "Release moves to Thursday"),
				sent("i3", "huck", "Release moves to Thursday"),
				sent("i4", "juno", "Rerun the Linux build"),
				ended("t1"),
				sent("i5", "juno", "Rerun the Linux build"),
			]),
		);
		const rows = grouped.filter((block) => block.kind === "inbox");
		expect(rows.map((block) => (block.kind === "inbox" ? [block.title, block.sent.map((one) => one.to)] : []))).toEqual([
			["Release moves to Thursday", ["juno", "ivy", "huck"]],
			["Rerun the Linux build", ["juno"]],
			["Rerun the Linux build", ["juno"]],
		]);
		// Named by its first item, so a member landing never makes it a new row.
		expect(rows[0]?.id).toBe("inbox-sent:i1");
		// What draws nothing still parts two turns' sends.
		expect(grouped.some((block) => block.kind === "event" && block.event.kind === "turn")).toBe(true);
	});

	test("a row says where its items stand, from the sender's live view", () => {
		const group = { title: "Release moves to Thursday", sent: [sent("i1", "juno", "x"), sent("i2", "ivy", "x"), sent("i3", "huck", "x")] as InboxSent[] };
		const known = (items: InboxItem[]) => new Map(items.map((one) => [one.id, one]));
		expect(inbox.sentState(group, new Map())).toBeUndefined();
		expect(inbox.sentState(group, known([item("i1", { state: "seen" }), item("i2", { state: "taken" }), item("i3")]))).toBe("2 of 3 seen");
		expect(inbox.sentState(group, known([item("i1", { state: "closed", outcome: "noted", receipts: [{ state: "seen", at: NOW }] }), item("i2", { state: "closed", outcome: "noted" }), item("i3", { state: "closed", outcome: "noted" })]))).toBe("Noted");
		const one = { title: "Rerun the Linux build", sent: [sent("i4", "juno", "x")] as InboxSent[] };
		expect(inbox.sentState(one, known([item("i4", { state: "taken" })]))).toBe("Taken");
		expect(inbox.sentState(one, known([item("i4", { state: "later", reason: "after the release" })]))).toBe("For later");
		expect(inbox.sentState(one, known([item("i4", { held: true, receipts: [] })]))).toBe("Awaiting approval");
		// One closed straight from its title, or dropped while held, was never seen.
		expect(inbox.wasSeen(item("x", { state: "closed", outcome: "dropped" }))).toBe(false);
		expect(inbox.wasSeen(item("x", { state: "closed", outcome: "done", receipts: [{ state: "taken", at: NOW }] }))).toBe(true);
		expect(inbox.toWords(["juno", "ivy", "huck", "chop"], nameOf)).toBe("Juno, Ivy, Huck +1");
	});
});

describe("task-456 the sorting card", () => {
	const entry = (id: string, more: Partial<InboxSortedEntry>): InboxSortedEntry => ({ item: id, title: `Title ${id}`, from: "chop", state: "taken", ...more });
	const sort = [
		entry("build", { state: "taken" }),
		entry("notes", { state: "later", reason: "after the build is green", from: "ivy" }),
		entry("thursday", { state: "closed", outcome: "noted" }),
		entry("red", { state: "closed", outcome: "combined", into: "build", from: "huck" }),
		entry("wipe", { state: "closed", outcome: "done", answered: true, from: "huck" }),
	];

	test("keeps the receiver's order, numbers what was taken or left for later, and sets a combined one in under the one it was folded into", () => {
		const rows = inbox.sortedRows(sort);
		expect(rows.map((row) => [row.entry.item, row.number ?? null, row.depth, row.word])).toEqual([
			["build", 1, 0, "Taken"],
			["red", null, 1, "Combined"],
			["notes", 2, 0, "Later"],
			["thursday", null, 0, "Noted"],
			["wipe", null, 0, "Answered"],
		]);
		expect(rows.filter((row) => row.taken).map((row) => row.entry.item)).toEqual(["build"]);
		expect(inbox.sortedCounts(sort)).toBe("1 taken · 1 for later · 3 closed");
		expect(inbox.readWords(5)).toBe("Read 5 messages");
		// Combined into one not in this sort, it stands where it was given.
		expect(inbox.sortedRows([entry("a", { state: "closed", outcome: "combined", into: "elsewhere" })]).map((row) => row.depth)).toEqual([0]);
		// Folded into each other, each still has a row.
		expect(inbox.sortedRows([entry("a", { state: "closed", outcome: "combined", into: "b" }), entry("b", { state: "closed", outcome: "combined", into: "a" })])).toHaveLength(2);
		expect(inbox.sortedWord(entry("x", { state: "closed" }))).toBe("Closed");
	});
});

describe("a turn the inbox woke is drawn as path", () => {
	const sortedLine: TranscriptEvent = {
		kind: "inbox_sorted",
		id: "inbox-sorted:1",
		ts: NOW,
		open: 0,
		unread: 0,
		entries: [{ item: "i1", title: "Test message", from: "chop", state: "closed", outcome: "done", answered: true }],
	};
	const tool = (id: string, title: string): TranscriptEvent => ({ kind: "tool", id, ts: NOW, toolCallId: id, title, status: "completed" });
	const thought = (id: string, text: string): TranscriptEvent => ({ kind: "thought", id, ts: NOW, text });

	test("its sorting card stays, its tools and words fold into the turn's steps, and no bubble follows", async () => {
		const { toBlocks } = await import("../src/transcriptFolds");
		const drawn = groupInbox(
			toBlocks([
				tool("t1", "read_inbox"),
				tool("t2", "update_inbox"),
				sent("r1", "chop", "Re: Test message"),
				sortedLine,
				thought("w1", "Pork Chop's test message arrived; I confirmed receipt."),
				ended("e1"),
			]),
		);
		expect(drawn.map((block) => (block.kind === "event" ? block.event.kind : block.kind))).toEqual(["inbox", "inbox_sorted", "steps", "turn"]);
		const steps = drawn.find((block) => block.kind === "steps");
		expect(steps?.kind === "steps" && steps.items.map((one) => one.kind)).toEqual(["tool", "tool", "thought"]);
		expect(drawn.some((block) => block.kind === "event" && block.event.kind === "agent")).toBe(false);
	});

	test("what it posted is the one bubble, with the turn's steps under it", async () => {
		const { toBlocks } = await import("../src/transcriptFolds");
		const drawn = groupInbox(
			toBlocks([
				tool("t1", "update_inbox"),
				sortedLine,
				tool("t2", "post_to_chat"),
				said("p1", "Pork Chop needs the crane permit signed: it is yours to sign."),
				thought("w1", "Sorted it and told the person."),
				ended("e1"),
			]),
		);
		expect(drawn.map((block) => (block.kind === "event" ? block.event.kind : block.kind))).toEqual(["inbox_sorted", "agent", "steps", "turn"]);
	});
});

describe("task-456 the pane", () => {
	test("groups what is open: not yet sorted, taken, for later; with the counts at its head", () => {
		const shown = view({
			open: [item("a"), item("b", { state: "seen" }), item("c", { state: "taken" }), item("d", { state: "later", reason: "after" }), item("e", { state: "noticed" })],
			unread: 2,
		});
		const groups = inbox.openGroups(shown);
		expect(groups.unread.map((one) => one.id)).toEqual(["a", "b", "e"]);
		expect(groups.taken.map((one) => one.id)).toEqual(["c"]);
		expect(groups.later.map((one) => one.id)).toEqual(["d"]);
		expect(inbox.openCounts(shown)).toBe("2 unread · 1 taken · 1 for later");
		// The composer's tray is only what the teammate has not read.
		expect(inbox.unreadItems(shown).map((one) => one.id)).toEqual(["a", "e"]);
	});

	test("an item's receipts are a timeline: sent, each move with its time, why, and what it was folded into", () => {
		const sentAt = NOW - 600_000;
		const rows = inbox.receiptRows(
			item("a", {
				ts: sentAt,
				receipts: [
					{ state: "new", at: sentAt + 2 },
					{ state: "noticed", at: sentAt + 4_000 },
					{ state: "seen", at: sentAt + 30_000 },
					{ state: "later", at: sentAt + 31_000, reason: "after the release" },
					{ state: "closed", at: sentAt + 90_000, outcome: "combined", into: "b" },
				],
			}),
			(id) => (id === "b" ? "Rerun the Linux build" : undefined),
		);
		expect(rows).toEqual([
			{ at: sentAt, word: "Sent" },
			{ at: sentAt + 4_000, word: "Noticed" },
			{ at: sentAt + 30_000, word: "Seen" },
			{ at: sentAt + 31_000, word: "Later", detail: "after the release" },
			{ at: sentAt + 90_000, word: "Combined", detail: "into Rerun the Linux build" },
		]);
		// Let in long after it was sent: the first-contact card's yes.
		expect(inbox.receiptRows(item("h", { ts: sentAt, receipts: [{ state: "new", at: sentAt + 120_000 }] })).map((row) => row.word)).toEqual(["Sent", "Allowed"]);
		expect(inbox.receiptRows(item("d", { ts: sentAt, receipts: [{ state: "closed", at: sentAt + 1, outcome: "dropped", reason: "The person stopped it" }] })).at(-1)).toEqual({
			at: sentAt + 1,
			word: "Dropped",
			detail: "The person stopped it",
		});
	});
});

let act: typeof import("react").act;
let root: Root | null;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

describe("task-456 drawn", () => {
	beforeEach(async () => {
		({ teardown } = installDom({ ResizeObserver: NoResize, requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} }));
		act = (await import("react")).act;
		const { createRoot } = await import("react-dom/client");
		container = document.createElement("div");
		document.body.append(container);
		root = createRoot(container);
	});
	afterEach(async () => {
		await act(async () => root?.unmount());
		root = null;
		await teardown();
	});
	const press = async (target: Element) => act(async () => void target.dispatchEvent(new window.MouseEvent("click", { bubbles: true, detail: 1 })));

	test("a sent row reads its state live and opens its first item", async () => {
		const { InboxRows, InboxSentRow } = await import("../src/components/transcript/InboxLines");
		const opened: string[] = [];
		const members = [sent("i1", "juno", "Release"), sent("i2", "ivy", "Release"), sent("i3", "huck", "Release")] as InboxSent[];
		const draw = (items: InboxItem[]) =>
			act(async () =>
				root!.render(
					<InboxRows.Provider value={{ work: new Set(), items: new Map(items.map((one) => [one.id, one])), onOpen: (id) => opened.push(id) }}>
						<InboxSentRow title="Release" sent={members} people={people} />
					</InboxRows.Provider>,
				),
			);
		await draw([item("i1"), item("i2"), item("i3")]);
		const row = () => container.querySelector<HTMLButtonElement>("button.link-row[data-kind=inbox]")!;
		expect(row().textContent).toContain("Release· To Juno, Ivy, Huck · Sent");
		await draw([item("i1", { state: "seen" }), item("i2", { state: "taken" }), item("i3")]);
		expect(row().textContent).toContain("To Juno, Ivy, Huck · 2 of 3 seen");
		await press(row());
		expect(opened).toEqual(["i1"]);
	});

	test("the sorting card draws the order, the numbers, the indent, waiting, and opens an item", async () => {
		const { InboxRows, InboxSortedCard } = await import("../src/components/transcript/InboxLines");
		const opened: string[] = [];
		const event = {
			kind: "inbox_sorted" as const,
			id: "sorted-1",
			ts: NOW,
			open: 2,
			unread: 0,
			entries: [
				{ item: "build", title: "Rerun the Linux build", from: "chop", state: "taken" as const },
				{ item: "notes", title: "Storage numbers", from: "ivy", state: "later" as const, reason: "after the build is green" },
				{ item: "thursday", title: "Release moves to Thursday", from: "chop", state: "closed" as const, outcome: "noted" as const },
				{ item: "red", title: "Linux build is red", from: "huck", state: "closed" as const, outcome: "combined" as const, into: "build" },
			],
		};
		await act(async () =>
			root!.render(
				<InboxRows.Provider value={{ work: new Set(), items: new Map([["build", item("build", { waiting: true, state: "taken" })]]), onOpen: (id) => opened.push(id) }}>
					<InboxSortedCard event={event} people={people} />
				</InboxRows.Provider>,
			),
		);
		const head = container.querySelector(".inbox-sorted-head")!;
		expect(head.textContent).toContain("Read 4 messages");
		expect(head.textContent).toContain("1 taken · 1 for later · 2 closed");
		const entries = [...container.querySelectorAll<HTMLLIElement>(".inbox-sorted li")];
		expect(entries.map((one) => one.querySelector(".list-title-text")?.textContent)).toEqual(["Rerun the Linux build", "Linux build is red", "Storage numbers", "Release moves to Thursday"]);
		expect(entries.map((one) => one.querySelector(".list-mark")?.textContent)).toEqual(["1", "", "2", "–"]);
		expect(entries.map((one) => one.dataset["depth"] ?? null)).toEqual([null, "1", null, null]);
		expect(entries[0]?.querySelector(".list-line")?.textContent).toBe("Pork Chop · waiting");
		expect(entries[0]?.querySelector(".list-when")?.hasAttribute("data-taken")).toBe(true);
		expect(entries[2]?.querySelector(".list-line")?.textContent).toBe("Ivy · after the build is green");
		await press(entries[1]!.querySelector("button")!);
		expect(opened).toEqual(["red"]);
	});

	test("a sort of one item is a quiet row, not a card", async () => {
		const { InboxSortedCard } = await import("../src/components/transcript/InboxLines");
		const event = { kind: "inbox_sorted" as const, id: "s", ts: NOW, open: 0, unread: 0, entries: [{ item: "x", title: "Nightly deploy is paused", from: "huck", state: "closed" as const, outcome: "noted" as const }] };
		await act(async () => root!.render(<InboxSortedCard event={event} people={people} />));
		expect(container.querySelector(".inbox-sorted")).toBeNull();
		expect(container.querySelector(".link-row[data-kind=inbox-sorted]")?.textContent).toBe("Nightly deploy is paused· From Huck · Noted");
	});

	test("the composer counts what the teammate has not read only while there is some, and its tray opens an item or the whole inbox", async () => {
		const { Composer } = await import("../src/components/Composer");
		const opened: string[] = [];
		const draw = (items: InboxItem[]) =>
			act(async () =>
				root!.render(
					<Composer
						personaId="juno"
						name="Juno"
						state="ready"
						replyQuote={null}
						onSend={() => {}}
						onCancel={() => {}}
						onClearReply={() => {}}
						inbox={{ items, nameOf, onOpen: (id) => opened.push(id), onAll: () => opened.push("all") }}
					/>,
				),
			);
		const key = () => container.querySelector<HTMLButtonElement>("button.composer-inbox");
		await draw([]);
		expect(key()).toBeNull();
		expect(container.querySelector(".composer[data-work]")).toBeNull();
		await draw([item("wipe", { from: "huck", waiting: true, title: "Is the staging database safe to wipe?" }, 40_000), item("log", { from: "ivy", title: "Changelog draft is ready" })]);
		expect(key()?.textContent).toBe("2 unread");
		expect(key()?.getAttribute("aria-expanded")).toBe("false");
		await press(key()!);
		const rows = [...container.querySelectorAll<HTMLButtonElement>(".composer-threads-panel .list-row[data-inbox]")];
		expect(rows.map((row) => row.querySelector(".list-line")?.textContent)).toEqual(["Huck · waiting", "Ivy"]);
		// The sender waiting is counted in amber; one not waiting says when it was sent.
		expect(rows[0]?.querySelector(".list-when")?.hasAttribute("data-waiting")).toBe(true);
		expect(rows[0]?.querySelector(".list-when")?.textContent).toMatch(/^0:4\d$/);
		expect(rows[1]?.querySelector(".list-when")?.hasAttribute("data-waiting")).toBe(false);
		await press(rows[0]!);
		expect(opened).toEqual(["wipe"]);
		expect(container.querySelector(".composer-threads-panel")).toBeNull();
		await press(key()!);
		await press(container.querySelector(".composer-threads-all")!);
		expect(opened).toEqual(["wipe", "all"]);
		// Read, the count goes.
		await draw([]);
		expect(key()).toBeNull();
	});

	test("the pane lists an inbox by segment, and an item's pane says its receipts, work thread and replies", async () => {
		const { InboxItemPane, InboxList } = await import("../src/components/InboxPane");
		const shown = view({
			open: [item("a", { title: "Is the staging database safe to wipe?", from: "huck", waiting: true }), item("t", { state: "taken", work: { kind: "side", key: "s1" } }), item("l", { state: "later", reason: "after the build" })],
			closed: [item("c", { state: "closed", outcome: "noted", title: "Nightly deploy is paused" })],
			sent: [item("s", { from: "juno", to: "chop", state: "seen", title: "Numbers are in" })],
			unread: 1,
		});
		const segments: string[] = [];
		const opened: string[] = [];
		const list = (segment: "open" | "closed" | "sent") =>
			act(async () =>
				root!.render(
					<InboxList name="Juno" view={shown} segment={segment} people={people} working={new Set(["s1"])} onSegment={(one) => segments.push(one)} onOpenItem={(id) => opened.push(id)} onClose={() => {}} />,
				),
			);
		await list("open");
		expect([...container.querySelectorAll(".list-group")].map((one) => one.textContent)).toEqual(["Unread1", "Taken1", "For later1"]);
		expect([...container.querySelectorAll("[data-inbox]")].map((one) => one.getAttribute("data-inbox"))).toEqual(["a", "t", "l"]);
		expect(container.querySelector("[data-inbox=a] .list-when")?.hasAttribute("data-waiting")).toBe(true);
		expect(container.querySelector("[data-inbox=t] .dock-dot")).not.toBeNull();
		expect(container.querySelector(".dock-note")?.textContent).toBe("1 unread · 1 taken · 1 for later");
		await press([...container.querySelectorAll("[role=tab]")].find((one) => one.textContent === "Sent")!);
		expect(segments).toEqual(["sent"]);
		await list("sent");
		expect(container.querySelector("[data-inbox=s] .list-line")?.textContent).toBe("To Pork Chop · Seen");
		await list("closed");
		expect(container.querySelector("[data-inbox=c] .list-line")?.textContent).toBe("Pork Chop · Noted");
		await press(container.querySelector("[data-inbox=c]")!);
		expect(opened).toEqual(["c"]);

		const sentAt = NOW - 300_000;
		const original = item("o", {
			ts: sentAt,
			title: "Rerun the Linux build",
			body: "The Linux build went red on the packaging step.",
			state: "closed",
			outcome: "done",
			work: { kind: "side", key: "s1" },
			receipts: [
				{ state: "new", at: sentAt },
				{ state: "seen", at: sentAt + 10_000 },
				{ state: "taken", at: sentAt + 20_000 },
				{ state: "closed", at: sentAt + 200_000, outcome: "done" },
			],
		});
		const reply = item("r", { from: "juno", to: "chop", title: "Re: Rerun the Linux build", replyTo: "o", wants: "nothing" });
		spyOn(wire, "command").mockImplementation((async (cmd: string, params: { id: string }) => {
			if (cmd !== "inbox.item") throw new Error(cmd);
			return params.id === "o" ? { item: original, replies: [reply] } : { item: reply, replies: [] };
		}) as typeof wire.command);
		const threads: string[] = [];
		await act(async () =>
			root!.render(
				<InboxItemPane itemId="o" people={people} onBack={() => {}} onClose={() => {}} onOpenItem={(id) => opened.push(id)} onOpenThread={(owner, thread) => threads.push(`${owner} ${thread.kind}:${thread.key}`)} />,
			),
		);
		expect(container.querySelector(".pane-title")?.textContent).toBe("Rerun the Linux build");
		expect(container.querySelector(".pane-meta")?.textContent).toBe("Pork Chop to Juno · wants an answer");
		expect(container.querySelector(".pane-task-body")?.textContent).toBe("The Linux build went red on the packaging step.");
		expect([...container.querySelectorAll(".inbox-receipts li .inbox-receipt-word")].map((one) => one.textContent)).toEqual(["Sent", "Seen", "Taken", "Done"]);
		// Nothing here sends, sorts or answers.
		expect(container.querySelector("textarea, input")).toBeNull();
		const rows = [...container.querySelectorAll<HTMLButtonElement>(".dock-page .list-row")];
		expect(rows.map((one) => one.querySelector(".list-title-text")?.textContent)).toEqual(["Rerun the Linux build", "Re: Rerun the Linux build"]);
		await press(rows[0]!);
		expect(threads).toEqual(["juno side:s1"]);
		await press(rows[1]!);
		expect(opened.at(-1)).toBe("r");
	});

	test("the receiver's conversation draws the work its sort started as a work-thread row, while an old handoff keeps its line", async () => {
		const { Transcript } = await import("../src/components/Transcript");
		const juno = view({ open: [item("i-build", { state: "taken", work: { kind: "side", key: "s-build" } })] });
		spyOn(wire, "subscribe").mockImplementation(((target: { inbox?: string }, handlers: { snapshot(items: unknown): void }) => {
			if (target.inbox === "juno") handlers.snapshot(juno);
			return () => {};
		}) as typeof wire.subscribe);
		const opened: string[] = [];
		const events: TranscriptEvent[] = [
			{ kind: "inbox_sorted", id: "sorted", ts: NOW - 2_000, open: 1, unread: 0, entries: [{ item: "i-build", title: "Rerun the Linux build", from: "chop", state: "taken" }] },
			{ kind: "link", id: "link:side:s-build", ts: NOW - 1_000, thread: "s-build", threadKind: "side", personaId: "juno", title: "Rerun the Linux build", state: "live", openerId: "chop", openerName: "Pork Chop" },
			{ kind: "link", id: "link:side:s-old", ts: NOW, thread: "s-old", threadKind: "side", personaId: "juno", title: "Old handoff", state: "live", openerId: "huck", openerName: "Huck" },
		];
		await act(async () =>
			root!.render(
				<Transcript personaId="juno" name="Juno" people={people} events={events} streaming={[]} live={false} focus={null} workingSides={["s-build", "s-old"]} onOpenThreads={() => {}} onOpenThread={(open) => opened.push(`${open.thread.kind}:${open.thread.key}`)} />,
			),
		);
		const row = container.querySelector<HTMLButtonElement>(".link-row[data-inbox]");
		expect(row?.querySelector(".link-row-title")?.textContent).toBe("Rerun the Linux build");
		expect(row?.querySelector(".link-row-outcome")?.textContent).toBe("· Work thread · Running");
		expect(row?.getAttribute("aria-label")).toBe("Work thread · Rerun the Linux build · Running. Open it");
		expect(row?.querySelector(".link-row-dot")?.getAttribute("data-tone")).toBe("live");
		await press(row!);
		expect(opened).toEqual(["side:s-build"]);
		// The handoff from before the inbox keeps its one line; this work is not in it.
		expect(container.textContent).toContain("Working on a handoff from Huck");
		expect(container.textContent).not.toContain("handoff from Pork Chop");
		expect(container.querySelectorAll(".link-row[data-kind=thread]")).toHaveLength(1);
	});

	test("a held item says it waits on the first-contact card", async () => {
		const { InboxItemPane } = await import("../src/components/InboxPane");
		spyOn(wire, "command").mockImplementation((async () => ({ item: item("h", { held: true, receipts: [] }), replies: [] })) as unknown as typeof wire.command);
		await act(async () => root!.render(<InboxItemPane itemId="h" people={people} onBack={() => {}} onClose={() => {}} onOpenItem={() => {}} onOpenThread={() => {}} />));
		expect(container.textContent).toContain("Held for the first-contact card.");
		expect([...container.querySelectorAll(".inbox-receipt-word")].map((one) => one.textContent)).toEqual(["Sent"]);
	});
});

describe("task-456 work an inbox item started is a work thread, not a handoff", () => {
	const started = (more: Partial<Extract<TranscriptEvent, { kind: "link" }>> = {}): Extract<TranscriptEvent, { kind: "link" }> => ({
		kind: "link", id: "link:side:s-build", ts: NOW, thread: "s-build", threadKind: "side", personaId: "juno", title: "Rerun the Linux build", state: "live", openerId: "chop", openerName: "Pork Chop", ...more,
	});
	const peopleNames = new Map([...people].map(([id, one]) => [id, one]));

	test("its row says its kind and state, on the receiver's tape and with whose it is on the sender's", async () => {
		const { inboxWorkLine, inboxWorkRow } = await import("../src/links");
		expect(inboxWorkRow(started(), "juno", peopleNames, true)).toEqual({ kind: "thread", title: "Rerun the Linux build", outcome: "Work thread · Running", tone: "live" });
		expect(inboxWorkRow(started(), "juno", peopleNames, false).outcome).toBe("Work thread · Open");
		expect(inboxWorkRow(started(), "chop", peopleNames, true).outcome).toBe("Work thread · Juno · Running");
		expect(inboxWorkRow(started({ state: "closed", outcome: "It was the runner" }), "juno", peopleNames).outcome).toBe("Work thread · It was the runner");
		expect(inboxWorkRow(started({ state: "parked" }), "juno", peopleNames).outcome).toBe("Work thread · Parked");
		expect(inboxWorkLine(started(), "juno", peopleNames, true)).toBe("Work thread · Rerun the Linux build · Running");
		expect(inbox.inboxWork(new Map([["b", item("b", { state: "taken", work: { kind: "side", key: "s-build" } })], ["n", item("n")]]))).toEqual(new Set(["s-build"]));
	});

	test("the Threads list and the composer's count list it as a work thread", async () => {
		const { listItems, markOf } = await import("../src/threadList");
		const { liveThreads } = await import("../src/liveThreads");
		const row = { thread: { kind: "side" as const, key: "s-build" }, personaId: "juno", title: "Rerun the Linux build", state: "live" as const, startedAt: NOW - 60_000, updatedAt: NOW, working: true, waiting: false, opener: { personaId: "chop", name: "Pork Chop" } };
		const old = { ...row, thread: { kind: "side" as const, key: "s-old" }, title: "Old handoff", opener: { personaId: "huck", name: "Huck" } };
		const items = listItems({ rows: [row, old], runs: [], jobs: [], asides: [], listedFor: "juno", nameOf, personName: "", now: NOW, inboxWork: new Set(["s-build"]) });
		expect(items.map((one) => [one.id, one.kind, markOf(one.kind).word])).toEqual([
			["side:s-build", "work", "Work thread"],
			["side:s-old", "handoff", "Handoff"],
		]);
		const juno = { persona: { id: "juno", name: "Juno" }, session: { personaId: "juno", state: "thinking" }, sides: [{ sideId: "s-build", title: "Rerun the Linux build", startedAt: NOW - 60_000, working: true }] } as unknown as import("../src/wire").RosterEntry;
		const live = liveThreads({ listed: [row], roster: [juno], entry: juno, jobs: [], nameOf, now: NOW, inboxWork: new Set(["s-build"]) });
		expect(live.map((one) => one.line)).toEqual(["Work thread from Pork Chop · Working"]);
		expect(live.some((one) => /handoff|handed/i.test(one.line))).toBe(false);
	});
});

describe("task-456 the way in", () => {
	test("the teammate's menu has Inbox beside Threads, with how many items it has not read", async () => {
		const { teammateActions } = await import("../src/components/Conversation");
		const actions = (unread: number) =>
			teammateActions({ inspectorOpen: false, chapterBusy: false, unseen: 0, unread, onToggleInspector: () => {}, onStartChapter: () => {}, onOpenThreadList: () => {}, onOpenInbox: () => {}, onDelete: () => {} });
		const ids = actions(0).flatMap((one) => (one.kind === "item" ? [one.id] : []));
		expect(ids.indexOf("inbox")).toBe(ids.indexOf("side-list") + 1);
		expect(actions(0).find((one) => one.kind === "item" && one.id === "inbox")).not.toHaveProperty("detail");
		expect(actions(3).find((one) => one.kind === "item" && one.id === "inbox")).toMatchObject({ text: "Inbox", detail: "3 unread" });
	});
});
