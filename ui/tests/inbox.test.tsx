import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { InboxItem, InboxSortedEntry, InboxView, TranscriptEvent } from "../src/generated/contract";
import type { InboxSent } from "../src/inbox";
import type { Block } from "../src/scheduledRuns";
import { installDom } from "./dom";

// task-456: the desk draws the inbox (docs/design/agent-inbox.md, "On the
// desk"). A sender's conversation has one quiet row per send, live from its
// inbox; a receiver's has one quiet line per sort; the composer counts what the
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
		// The person reads Taken as Picked up; the stored state stays taken.
		expect(inbox.sentState(one, known([item("i4", { state: "taken" })]))).toBe("Picked up");
		expect(inbox.sentState(one, known([item("i4", { state: "later", reason: "after the release" })]))).toBe("Later");
		expect(inbox.sentState(one, known([item("i4", { held: true, receipts: [] })]))).toBe("Awaiting approval");
		// One closed straight from its title, or dropped while held, was never seen.
		expect(inbox.wasSeen(item("x", { state: "closed", outcome: "dropped" }))).toBe(false);
		expect(inbox.wasSeen(item("x", { state: "closed", outcome: "done", receipts: [{ state: "taken", at: NOW }] }))).toBe(true);
		expect(inbox.toWords(["juno", "ivy", "huck", "chop"], nameOf)).toBe("Juno, Ivy, Huck +1");
	});
});

describe("task-456 the sort's line", () => {
	const entry = (id: string, more: Partial<InboxSortedEntry>): InboxSortedEntry => ({ item: id, title: `Title ${id}`, from: "chop", state: "taken", ...more });

	test("says what became of each in a word", () => {
		expect(
			[
				entry("build", { state: "taken" }),
				entry("notes", { state: "later", reason: "after the build is green" }),
				entry("thursday", { state: "closed", outcome: "noted" }),
				entry("red", { state: "closed", outcome: "combined", into: "build" }),
				entry("wipe", { state: "closed", outcome: "answered" }),
				entry("rerun", { state: "closed", outcome: "done" }),
				entry("spam", { state: "closed", outcome: "dropped" }),
				entry("x", { state: "closed" }),
			].map(inbox.sortedWord),
		).toEqual(["Picked up", "Later", "Noted", "Combined", "Answered", "Done", "Dropped", "Closed"]);
	});
});

describe("one word for each outcome", () => {
	test("a reply that went back is Answered and finished work is Done, the same in the chat row, the sort's row and the pane", () => {
		const answered = item("a", { state: "closed", outcome: "answered" });
		const done = item("d", { state: "closed", outcome: "done" });
		const one = (id: string) => ({ title: "x", sent: [sent(id, "juno", "x")] as InboxSent[] });
		const known = new Map([answered, done].map((one) => [one.id, one]));
		expect([inbox.sentState(one("a"), known), inbox.stateWord(answered), inbox.sortedWord({ item: "a", title: "x", from: "chop", state: "closed", outcome: "answered" })]).toEqual(["Answered", "Answered", "Answered"]);
		expect([inbox.sentState(one("d"), known), inbox.stateWord(done), inbox.sortedWord({ item: "d", title: "x", from: "chop", state: "closed", outcome: "done" })]).toEqual(["Done", "Done", "Done"]);
	});

	test("noticed, the desk telling the receiver an item exists, reads as Sent, and is no row of its own in the receipts", () => {
		const sentAt = NOW - 60_000;
		const noticed = item("n", { ts: sentAt, state: "noticed", receipts: [{ state: "new", at: sentAt }, { state: "noticed", at: sentAt + 5_000 }] });
		expect(inbox.stateWord(noticed)).toBe("Sent");
		expect(inbox.receiptRows(noticed).map((row) => row.word)).toEqual(["Sent"]);
	});
});

describe("a turn the inbox woke is drawn as path", () => {
	const sortedLine: TranscriptEvent = {
		kind: "inbox_sorted",
		id: "inbox-sorted:1",
		ts: NOW,
		open: 0,
		unread: 0,
		entries: [{ item: "i1", title: "Test message", from: "chop", state: "closed", outcome: "answered" }],
	};
	const tool = (id: string, title: string): TranscriptEvent => ({ kind: "tool", id, ts: NOW, toolCallId: id, title, status: "completed" });
	const thought = (id: string, text: string): TranscriptEvent => ({ kind: "thought", id, ts: NOW, text });

	test("its sort's line stays, its tools and words fold into the turn's steps, and no bubble follows", async () => {
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

	test("a sort in the middle of a turn the person started is one line where it fell, and the turn's steps stay one run under the reply", async () => {
		const { toBlocks } = await import("../src/transcriptFolds");
		const drawn = groupInbox(
			toBlocks([
				{ kind: "user", id: "u1", ts: NOW, text: "What changed in Postgres 18 for logical replication?" },
				tool("t1", "web_search: postgres 18 logical replication"),
				tool("t2", "read_inbox"),
				sent("r1", "huck", "Re: Is the staging database safe to wipe?"),
				tool("t3", "update_inbox"),
				sortedLine,
				tool("t4", "Fetch https://www.postgresql.org/docs/18/release-18.html"),
				said("a1", "Yes, mostly."),
				ended("e1"),
			]),
		);
		expect(drawn.map((block) => (block.kind === "event" ? block.event.kind : block.kind))).toEqual(["user", "inbox", "inbox_sorted", "agent", "steps", "turn"]);
		const steps = drawn.filter((block) => block.kind === "steps");
		expect(steps).toHaveLength(1);
		expect(steps[0]?.kind === "steps" && steps[0].items.map((one) => one.id)).toEqual(["t1", "t2", "t3", "t4"]);
	});
});

describe("task-494 a run of inbox activity is one line in the chat", () => {
	const tool = (id: string, title: string): TranscriptEvent => ({ kind: "tool", id, ts: NOW, toolCallId: id, title, status: "completed" });
	const quietEnd = (id: string): TranscriptEvent => ({ kind: "turn", id, ts: NOW, stopReason: "end_turn", quiet: true });
	const sorted = (id: string, ...items: string[]): TranscriptEvent => ({
		kind: "inbox_sorted",
		id,
		ts: NOW,
		open: 0,
		unread: 0,
		entries: items.map((one) => ({ item: one, title: `Title ${one}`, from: "chop", state: "closed" as const, outcome: "noted" as const })),
	});
	const link = (id: string, thread: string, state: "live" | "closed"): TranscriptEvent =>
		({ kind: "link", id, ts: NOW, thread, threadKind: "side", title: "Rerun the Linux build", state, personaId: "juno", openerId: "chop" }) as TranscriptEvent;
	const shape = (blocks: Block[]) =>
		blocks.flatMap((block) =>
			block.kind === "inboxRun"
				? [`run(${block.read},${block.sent}):${block.members.map((member) => (member.kind === "event" ? member.event.id : member.kind === "steps" ? `steps:${member.id}` : member.id)).join(",")}`]
				: block.kind === "event"
					? block.event.kind === "turn"
						? []
						: [block.event.id]
					: block.kind === "inbox"
						? [`${block.id}${block.asked === true ? " asked" : ""}`]
						: [`${block.kind}:${block.id}`],
		);
	const fold = async (events: TranscriptEvent[], work: ReadonlySet<string> = new Set()) => {
		const { toBlocks } = await import("../src/transcriptFolds");
		const { foldInboxRuns } = await import("../src/scheduledRuns");
		return foldInboxRuns(groupInbox(toBlocks(events)), work);
	};

	test("quiet turns' sorts, sends and times run together into one line, which says how many it read and sent", async () => {
		const drawn = await fold([
			tool("t1", "read_inbox"),
			sent("r1", "chop", "Re: Test message"),
			sorted("s1", "i1"),
			quietEnd("e1"),
			tool("t2", "read_inbox"),
			sorted("s2", "i2", "i3"),
			quietEnd("e2"),
		]);
		expect(shape(drawn)).toEqual(["run(3,1):inbox-sent:r1,s1,steps:t1,s2,steps:t2"]);
		// Named by its first row, so it keeps its row as it grows.
		expect(drawn[0]?.kind === "inboxRun" && drawn[0].id).toBe("inbox-run:inbox-sent:r1");
		expect(inbox.runWords(3, 1)).toBe("Inbox · read 3, sent 1");
	});

	test("the label says only what the run has: only sorting, only sending", () => {
		expect(inbox.runWords(2, 0)).toBe("Inbox · read 2");
		expect(inbox.runWords(0, 1)).toBe("Inbox · sent 1");
	});

	test("a run ends at the person's line, a bubble the teammate posts and a turn the person asked for", async () => {
		const drawn = await fold([
			tool("t1", "read_inbox"),
			sorted("s1", "i1"),
			{ kind: "tool", id: "t2", ts: NOW, toolCallId: "t2", title: "post_to_chat", status: "completed" },
			said("p1", "The crane permit is yours to sign."),
			quietEnd("e1"),
			tool("t3", "read_inbox"),
			sorted("s2", "i2"),
			quietEnd("e2"),
			{ kind: "user", id: "u1", ts: NOW, text: "Ask Huck whether staging is safe to wipe." },
			sent("a1", "huck", "Is staging safe to wipe?"),
			tool("t4", "update_inbox"),
			sorted("s3", "i3"),
			said("a2", "Asked Huck."),
			ended("e3"),
			tool("t5", "read_inbox"),
			sorted("s4", "i4"),
			quietEnd("e4"),
		]);
		expect(shape(drawn)).toEqual([
			"run(1,0):s1",
			"p1",
			// A quiet turn that posted keeps its time under its bubble, as any reply does.
			"steps:t1",
			"run(1,0):s2,steps:t3",
			"u1",
			// What came of the person's ask stays out of the line, a quiet row of its own.
			"inbox-sent:a1 asked",
			"run(1,0):s3",
			"a2",
			"steps:t4",
			"run(1,0):s4,steps:t5",
		]);
	});

	test("a work thread an item started stands beside the run while it runs, and folds in once it is over", async () => {
		const work = new Set(["w1"]);
		const running = await fold([tool("t1", "update_inbox"), sorted("s1", "i1"), link("l1", "w1", "live"), quietEnd("e1"), tool("t2", "read_inbox"), sorted("s2", "i2"), quietEnd("e2")], work);
		// It does not split the run: the line, then the running thread under it.
		expect(shape(running)).toEqual(["run(2,0):s1,steps:t1,s2,steps:t2", "l1"]);
		const over = await fold([tool("t1", "update_inbox"), sorted("s1", "i1"), link("l1", "w1", "closed"), quietEnd("e1"), tool("t2", "read_inbox"), sorted("s2", "i2"), quietEnd("e2")], work);
		expect(shape(over)).toEqual(["run(2,0):s1,l1,steps:t1,s2,steps:t2"]);
		// A thread no item started is no part of a run.
		expect(shape(await fold([sorted("s1", "i1"), link("l1", "w1", "closed"), sorted("s2", "i2")]))).toEqual(["run(1,0):s1", "l1", "run(1,0):s2"]);
	});

	test("a quiet turn's time alone, with nothing read or sent, stays its own line", async () => {
		expect(shape(await fold([tool("t1", "read_inbox"), quietEnd("e1")]))).toEqual(["steps:t1"]);
	});

	test("a run that grows keeps the block it was, and the turn still running sits under it", async () => {
		const { keepBlocks } = await import("../src/transcriptFolds");
		const first = await fold([tool("t1", "update_inbox"), sorted("s1", "i1"), quietEnd("e1"), tool("t2", "read_inbox")]);
		expect(shape(first)).toEqual(["run(1,0):s1,steps:t1", "steps:t2"]);
		const grown = await fold([tool("t1", "update_inbox"), sorted("s1", "i1"), quietEnd("e1"), tool("t2", "read_inbox"), sorted("s2", "i2"), quietEnd("e2")]);
		expect(grown[0]?.kind === "inboxRun" && grown[0].id).toBe(first[0]?.kind === "inboxRun" && first[0].id);
		expect(keepBlocks(first, first)[0]).toBe(first[0]);
	});
});

describe("the three lists", () => {
	test("one send to several teammates is one send, however its items' states differ; another send of the same title is another", () => {
		const to = (id: string, who: string, others: string[], more: Partial<InboxItem> = {}) => item(id, { from: "juno", to: who, alsoTo: others, title: "Release moves to Thursday", ...more });
		// Newest first, as the view lists them: the send made t1, t2, t3 in that order.
		const listed = [
			to("t3", "chop", ["huck", "ivy"]),
			to("t2", "ivy", ["huck", "chop"], { state: "taken" }),
			to("t1", "huck", ["ivy", "chop"], { state: "seen" }),
			to("t4", "huck", ["ivy", "chop"]),
			item("one", { from: "juno", to: "huck", title: "Rerun the Linux build" }),
		];
		const grouped = inbox.sends(listed);
		expect(grouped.map((send) => send.items.map((one) => one.id))).toEqual([["t1", "t2", "t3"], ["t4"], ["one"]]);
		expect(inbox.sendState(grouped[0]!)).toBe("2 of 3 seen");
		expect(inbox.listHeading(view({ sent: listed }), "sent")).toBe("3 sent");
		expect(inbox.listHeading(view(), "sent")).toBe("Nothing sent");
		expect(inbox.listHeading(view(), "closed")).toBe("Nothing closed");
		expect(inbox.listHeading(view(), "open")).toBe("Nothing open");
	});
});

describe("a teammate woken by its inbox", () => {
	test("a turn the person was not part of ends Worked quietly in the steps pane, never Done; a stopped one is Stopped", async () => {
		const { runEnding } = await import("../src/transcriptFolds");
		const step: TranscriptEvent = { kind: "tool", id: "t1", ts: NOW, toolCallId: "t1", title: "update_inbox", status: "completed" };
		const items = [{ kind: "tool", id: "t1" }] as unknown as import("../src/scheduledRuns").Step[];
		const end = (more: Partial<Extract<TranscriptEvent, { kind: "turn" }>>): TranscriptEvent => ({ kind: "turn", id: "e1", ts: NOW, stopReason: "end_turn", ...more });
		expect(runEnding([step, end({ quiet: true })], items)).toBe("quiet");
		expect(runEnding([step, end({})], items)).toBe("done");
		expect(runEnding([step, end({ quiet: true, stopReason: "cancelled" })], items)).toBe("stopped");
	});
});

describe("task-456 the pane", () => {
	test("groups what is open: not yet sorted, picked up, for later; with the counts at its head", () => {
		const shown = view({
			open: [item("a"), item("b", { state: "seen" }), item("c", { state: "taken" }), item("d", { state: "later", reason: "after" }), item("e", { state: "noticed" })],
			unread: 2,
		});
		const groups = inbox.openGroups(shown);
		expect(groups.unread.map((one) => one.id)).toEqual(["a", "b", "e"]);
		expect(groups.taken.map((one) => one.id)).toEqual(["c"]);
		expect(groups.later.map((one) => one.id)).toEqual(["d"]);
		expect(inbox.listHeading(shown, "open")).toBe("2 unread · 1 picked up · 1 for later");
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

describe("an item that expired", () => {
	test("says Expired wherever an outcome is worded: its state, its sort's row and its receipts", () => {
		const sentAt = NOW - 3 * 86_400_000;
		const expired = item("x", {
			ts: sentAt,
			state: "closed",
			outcome: "expired",
			reason: "Not picked up in 3 days.",
			receipts: [
				{ state: "new", at: sentAt },
				{ state: "later", at: sentAt + 60_000, reason: "after the release" },
				{ state: "closed", at: NOW, outcome: "expired", reason: "Not picked up in 3 days." },
			],
		});
		expect(inbox.outcomeWord("expired")).toBe("Expired");
		expect(inbox.stateWord(expired)).toBe("Expired");
		expect(inbox.sortedWord({ item: "x", title: "Title x", from: "chop", state: "closed", outcome: "expired" })).toBe("Expired");
		expect(inbox.receiptRows(expired).at(-1)).toEqual({ at: NOW, word: "Expired", detail: "Not picked up in 3 days." });
	});

	test("Hotline's notice of it is from Hotline, by its mark and not its words", () => {
		const notice = item("n", { from: "juno", to: "chop", title: "Expired: Title x", wants: "nothing", replyTo: "x", notice: "expired" });
		expect(inbox.senderWords(notice, nameOf)).toBe("Hotline");
		// A teammate's own title that says "Expired" is still the teammate's.
		expect(inbox.senderWords(item("f", { title: "Expired: Title x" }), nameOf)).toBe("Pork Chop");
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
					<InboxRows.Provider value={{ work: new Map(), items: new Map(items.map((one) => [one.id, one])), onOpen: (id) => opened.push(id) }}>
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

	/** A run as the fold makes it, of these lines. */
	const runOf = async (events: TranscriptEvent[], work: ReadonlySet<string> = new Set()) => {
		const { toBlocks } = await import("../src/transcriptFolds");
		const { foldInboxRuns } = await import("../src/scheduledRuns");
		const run = foldInboxRuns(groupInbox(toBlocks(events)), work).find((block) => block.kind === "inboxRun");
		if (run?.kind !== "inboxRun") throw new Error("no run");
		return run;
	};

	test("a run is one quiet line that opens in place to its rows in the order they happened, each item's opening it", async () => {
		const { InboxRows, InboxRun } = await import("../src/components/transcript/InboxLines");
		const opened: string[] = [];
		const worked: string[] = [];
		const run = await runOf([
			{
				kind: "inbox_sorted",
				id: "sorted-1",
				ts: NOW,
				open: 1,
				unread: 0,
				entries: [
					{ item: "thursday", title: "Release moves to Thursday", from: "ivy", state: "closed", outcome: "noted" },
					{ item: "notes", title: "Storage numbers", from: "chop", state: "later", reason: "after this research" },
				],
			},
			sent("r1", "huck", "Re: Is the staging database safe to wipe?"),
			{ kind: "tool", id: "t1", ts: NOW, toolCallId: "t1", title: "update_inbox", status: "completed" },
			{ kind: "turn", id: "e1", ts: NOW, stopReason: "end_turn", quiet: true },
		]);
		await act(async () =>
			root!.render(
				<InboxRows.Provider value={{ work: new Map(), items: new Map([["r1", item("r1", { to: "huck", state: "seen" })]]), onOpen: (id) => opened.push(id) }}>
					<InboxRun run={run} owner="juno" people={people} onOpenWork={(id) => worked.push(id)} />
				</InboxRows.Provider>,
			),
		);
		// A path line, not a card: no fact, no rows until it is pressed.
		const line = container.querySelector<HTMLButtonElement>("[data-kind=inbox-run] .trail-step > .trail-head")!;
		expect(line.querySelector(".trail-said")?.textContent).toBe("Inbox · read 2, sent 1");
		expect(line.getAttribute("aria-expanded")).toBe("false");
		expect(container.querySelector(".trail-fact")).toBeNull();
		expect(container.querySelectorAll(".link-row")).toHaveLength(0);
		await press(line);
		// No rule or box around them: the conversation's own quiet rows, in order, the quiet turn's time last.
		const rows = container.querySelector(".inbox-run-rows")!;
		expect(container.querySelector(".trail-detail")).toBeNull();
		expect([...rows.querySelectorAll(".link-row")].map((row) => row.textContent)).toEqual([
			"Release moves to Thursday· From Ivy · NotedOpen ›",
			"Storage numbers· From Pork Chop · Later · after this researchOpen ›",
			"Re: Is the staging database safe to wipe?· To Huck · SeenOpen ›",
		]);
		expect([...rows.querySelectorAll(".link-row")].every((row) => row.closest(".inbox-run-rows") === rows)).toBe(true);
		expect(rows.lastElementChild?.querySelector(".steps-caption")).not.toBeNull();
		await press(rows.querySelectorAll<HTMLButtonElement>(".link-row")[1]!);
		await press(rows.querySelectorAll<HTMLButtonElement>(".link-row")[2]!);
		expect(opened).toEqual(["notes", "r1"]);
		await press(rows.querySelector<HTMLButtonElement>(".steps-caption")!);
		expect(worked).toHaveLength(1);
		await press(line);
		expect(container.querySelectorAll(".link-row")).toHaveLength(0);
	});

	test("Picked up in the chat's inbox rows: a sort's row and a sent row say it where the item was taken", async () => {
		const { InboxRows, InboxRun, InboxSentRow } = await import("../src/components/transcript/InboxLines");
		const run = await runOf([
			{ kind: "inbox_sorted", id: "s", ts: NOW, open: 1, unread: 0, entries: [{ item: "build", title: "Rerun the Linux build", from: "chop", state: "taken" }] },
			{ kind: "turn", id: "e", ts: NOW, stopReason: "end_turn", quiet: true },
		]);
		const taken = item("i1", { state: "taken" });
		await act(async () =>
			root!.render(
				<InboxRows.Provider value={{ work: new Map(), items: new Map([["i1", taken]]), onOpen: () => {} }}>
					<InboxRun run={run} owner="juno" people={people} onOpenWork={() => {}} />
					<InboxSentRow title="Release" sent={[sent("i1", "juno", "Release")] as InboxSent[]} people={people} />
				</InboxRows.Provider>,
			),
		);
		expect(container.querySelector("button.link-row[data-kind=inbox]")?.textContent).toContain("To Juno · Picked up");
		await press(container.querySelector<HTMLButtonElement>("[data-kind=inbox-run] .trail-step > .trail-head")!);
		expect(container.querySelector(".inbox-run-rows .link-row")?.textContent).toContain("From Pork Chop · Picked up");
		expect(container.textContent).not.toContain("Taken");
	});

	test("a run's row for Hotline's notice credits Hotline, and an item's row does nothing where nothing opens it", async () => {
		const { InboxRun } = await import("../src/components/transcript/InboxLines");
		const run = await runOf([
			{
				kind: "inbox_sorted",
				id: "s",
				ts: NOW,
				open: 0,
				unread: 0,
				entries: [{ item: "n", title: "Expired: Changelog wording", from: "huck", state: "closed", outcome: "noted", notice: "expired" }],
			},
		]);
		await act(async () => root!.render(<InboxRun run={run} owner="juno" people={people} />));
		const line = container.querySelector<HTMLButtonElement>("[data-kind=inbox-run] .trail-head")!;
		expect(line.textContent).toBe("Inbox · read 1");
		await press(line);
		const row = container.querySelector(".inbox-run-rows .link-row")!;
		expect(row.tagName).toBe("P");
		expect(row.textContent).toBe("Expired: Changelog wording· From Hotline · Noted");
	});

	test("the composer counts what the teammate has not read only while there is some, its tray says when each arrived, and opens an item or the whole inbox", async () => {
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
		// A message whose sender is waiting says when it arrived, as every row does: "waiting" is in its line, not an amber count.
		const { asideTime } = await import("../src/asides");
		expect(rows.map((row) => row.querySelector(".list-when")?.textContent)).toEqual([asideTime(NOW - 40_000), asideTime(NOW - 60_000)]);
		expect(rows.some((row) => row.querySelector(".list-when")?.hasAttribute("data-waiting"))).toBe(false);
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
			closed: [item("c", { state: "closed", outcome: "noted", title: "Nightly deploy is paused" }), item("x", { state: "closed", outcome: "expired", title: "Old nightly artifacts" })],
			sent: [item("s", { from: "juno", to: "chop", state: "seen", title: "Numbers are in" })],
			unread: 1,
		});
		const segments: string[] = [];
		const opened: string[] = [];
		const threads: string[] = [];
		const list = (segment: "open" | "closed" | "sent") =>
			act(async () =>
				root!.render(
					<InboxList name="Juno" view={shown} segment={segment} people={people} working={new Set(["s1"])} onSegment={(one) => segments.push(one)} onOpenItem={(id) => opened.push(id)} onOpenThread={(owner, thread) => threads.push(`${owner} ${thread.kind}:${thread.key}`)} onClose={() => {}} />,
				),
			);
		await list("open");
		// Each group's name on a rule, its count at the rule's end; what was taken reads Picked up.
		expect([...container.querySelectorAll(".list-group")].map((one) => one.textContent)).toEqual(["Unread1", "Picked up1", "For later1"]);
		expect(container.querySelector(".list-group .spine-rule-line")).not.toBeNull();
		expect([...container.querySelectorAll("[data-inbox]")].map((one) => one.getAttribute("data-inbox"))).toEqual(["a", "t", "l"]);
		// A waiting sender is said in the line, and the row's time is a time, as every row's is: no amber countdown.
		expect(container.querySelector("[data-inbox=a] .list-line")?.textContent).toBe("Huck · waiting");
		const { asideTime } = await import("../src/asides");
		expect(container.querySelector("[data-inbox=a] .list-when")?.textContent).toBe(asideTime(NOW - 60_000));
		expect(container.querySelector("[data-inbox=a] .list-when")?.hasAttribute("data-waiting")).toBe(false);
		expect(container.querySelector("[data-inbox=t] .dock-dot")).not.toBeNull();
		// The picked-up item's work thread is a row under it, on the text edge with no mark of its own, which opens the thread.
		expect(container.querySelector("[data-work=t]")?.textContent).toBe("Work thread · Running");
		expect(container.querySelector("[data-work=t] .list-mark")).toBeNull();
		expect(container.querySelector("[data-inbox=t]")?.hasAttribute("data-live")).toBe(true);
		expect(container.querySelectorAll("[data-work]")).toHaveLength(1);
		await press(container.querySelector("[data-work=t]")!);
		expect(threads).toEqual(["juno side:s1"]);
		threads.length = 0;
		// The count line stays above the tabs.
		expect(container.querySelector(".spine-count")?.textContent).toBe("1 unread · 1 picked up · 1 for later");
		expect(container.querySelector(".spine-count + .list-span [role=tablist]")).not.toBeNull();
		await press([...container.querySelectorAll("[role=tab]")].find((one) => one.textContent === "Sent")!);
		expect(segments).toEqual(["sent"]);
		await list("sent");
		expect(container.querySelector("[data-inbox=s] .list-line")?.textContent).toBe("To Pork Chop · Seen");
		// The heading follows the tab, and a sent row's arrow never takes its receiver's state's colour.
		expect(container.querySelector(".spine-count")?.textContent).toBe("1 sent");
		expect(container.querySelector("[data-inbox=s]")?.hasAttribute("data-state")).toBe(false);
		await list("closed");
		expect(container.querySelector(".spine-count")?.textContent).toBe("2 closed");
		expect(container.querySelector("[data-inbox=c] .list-line")?.textContent).toBe("Pork Chop · Noted");
		expect(container.querySelector("[data-inbox=c] .list-mark")?.textContent).toBe("✓");
		expect(container.querySelector("[data-inbox=x] .list-mark")?.textContent).toBe("◌");
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
		await act(async () =>
			root!.render(
				<InboxItemPane itemId="o" people={people} onBack={() => {}} onClose={() => {}} onOpenItem={(id) => opened.push(id)} onOpenThread={(owner, thread) => threads.push(`${owner} ${thread.kind}:${thread.key}`)} />,
			),
		);
		// Where it stands is the state line under its title.
		expect(container.querySelector(".spine-title")?.textContent).toBe("Rerun the Linux build");
		expect(container.querySelector(".spine-state-word")?.textContent).toBe("Done");
		// No fact rows: who and what stand at the end of the message's rule.
		expect(container.querySelector(".spine-facts")).toBeNull();
		const message = container.querySelector('section[aria-label="Message"] .spine-rule');
		expect(message?.querySelector(".spine-rule-title")?.textContent).toBe("Message");
		expect(message?.querySelector(".spine-rule-aside")?.textContent).toBe("Pork Chop to Juno · wants an answer");
		expect(container.querySelector(".spine-who")).toBeNull();
		expect(container.querySelector('section[aria-label="Message"] .spine-said')?.textContent).toBe("The Linux build went red on the packaging step.");
		expect([...container.querySelectorAll(".inbox-receipts li .inbox-receipt-word")].map((one) => one.textContent)).toEqual(["Sent", "Seen", "Picked up", "Done"]);
		// Each receipt hangs on the spine: done ones filled, the last one as the item ended.
		expect([...container.querySelectorAll(".inbox-receipts li > .spine-mark")].map((one) => one.getAttribute("data-mark"))).toEqual(["done", "done", "done", "done"]);
		// Nothing here sends, sorts or answers.
		expect(container.querySelector("textarea, input")).toBeNull();
		const rows = [...container.querySelectorAll<HTMLButtonElement>(".dock-page .list-row")];
		expect(rows.map((one) => one.querySelector(".list-title-text")?.textContent)).toEqual(["Rerun the Linux build", "Re: Rerun the Linux build"]);
		await press(rows[0]!);
		expect(threads).toEqual(["juno side:s1"]);
		await press(rows[1]!);
		expect(opened.at(-1)).toBe("r");
	});

	test("an item a newer Hotline saved is a plain row where it belongs, saying so, that opens nothing", async () => {
		const { InboxList } = await import("../src/components/InboxPane");
		const newer = (id: string, more: Partial<import("../src/generated/contract").InboxNewerItem>) => ({ id, ts: NOW - 120_000, from: "chop", to: "juno", title: `Title ${id}`, state: "parked", ...more });
		const shown = view({
			open: [item("a")],
			unread: 1,
			newer: [newer("n-open", {}), newer("n-closed", { state: "closed" }), newer("n-sent", { from: "juno", to: "huck" })],
		});
		const opened: string[] = [];
		const draw = (segment: "open" | "closed" | "sent") =>
			act(async () => root!.render(<InboxList name="Juno" view={shown} segment={segment} people={people} working={new Set()} onSegment={() => {}} onOpenItem={(id) => opened.push(id)} onOpenThread={() => {}} onClose={() => {}} />));
		await draw("open");
		expect([...container.querySelectorAll("[data-newer]")].map((one) => one.getAttribute("data-newer"))).toEqual(["n-open"]);
		expect(container.querySelector(".spine-count")?.textContent).toBe("1 unread · 1 from a newer version");
		const row = container.querySelector("[data-newer=n-open]")!;
		expect(row.tagName).toBe("DIV");
		expect(row.querySelector(".list-line")?.textContent).toBe("Pork Chop · Saved by a newer version of Hotline. It can't be changed here.");
		// On Open it stands under its own heading on a rule, not under the group before it.
		expect(row.previousElementSibling?.textContent).toBe("From a newer version1");
		await draw("closed");
		expect([...container.querySelectorAll("[data-newer]")].map((one) => one.getAttribute("data-newer"))).toEqual(["n-closed"]);
		expect(container.querySelector(".spine-empty")).toBeNull();
		expect(container.querySelector(".spine-count")?.textContent).toBe("1 closed");
		await draw("sent");
		expect(container.querySelector("[data-newer=n-sent] .list-line")?.textContent).toBe("To Huck · Saved by a newer version of Hotline. It can't be changed here.");
		expect(opened).toEqual([]);
	});

	test("on Sent, one send to three teammates is one row that opens to a row for each, each opening its item", async () => {
		const { InboxList } = await import("../src/components/InboxPane");
		const to = (id: string, who: string, others: string[], more: Partial<InboxItem> = {}) => item(id, { from: "juno", to: who, alsoTo: others, title: "Release moves to Thursday", ...more });
		const shown = view({ sent: [to("t3", "chop", ["huck", "ivy"], { state: "taken" }), to("t2", "ivy", ["huck", "chop"], { state: "noticed" }), to("t1", "huck", ["ivy", "chop"], { state: "seen" })] });
		const opened: string[] = [];
		const draw = () => act(async () => root!.render(<InboxList name="Juno" view={shown} segment="sent" people={people} working={new Set()} onSegment={() => {}} onOpenItem={(id) => opened.push(id)} onOpenThread={() => {}} onClose={() => {}} />));
		await draw();
		const send = container.querySelector<HTMLButtonElement>("[data-send=t1]")!;
		expect(container.querySelectorAll(".list-row")).toHaveLength(1);
		expect(send.querySelector(".list-line")?.textContent).toBe("To Huck, Ivy, Pork Chop · 2 of 3 seen");
		expect(send.getAttribute("aria-expanded")).toBe("false");
		await press(send);
		expect(send.getAttribute("aria-expanded")).toBe("true");
		const each = [...container.querySelectorAll<HTMLButtonElement>(".list-row-sub")];
		expect(each.map((row) => row.textContent)).toEqual(["To Huck · Seen", "To Ivy · Sent", "To Pork Chop · Picked up"]);
		await press(each[2]!);
		expect(opened).toEqual(["t3"]);
		await press(send);
		expect(container.querySelectorAll(".list-row-sub")).toHaveLength(0);
	});

	test("an item's pane says who else the send reached, and nothing for one sent to one; with no words, its who and what stand under the state line", async () => {
		const { InboxItemPane } = await import("../src/components/InboxPane");
		const shared = item("shared", { alsoTo: ["ivy", "huck"], title: "Release moves to Thursday" });
		const alone = item("alone");
		const bare = item("bare", { body: "" });
		spyOn(wire, "command").mockImplementation((async (_cmd: string, params: { id: string }) => ({ item: params.id === "shared" ? shared : params.id === "bare" ? bare : alone, replies: [] })) as typeof wire.command);
		const draw = (itemId: string) => act(async () => root!.render(<InboxItemPane key={itemId} itemId={itemId} people={people} onBack={() => {}} onClose={() => {}} onOpenItem={() => {}} onOpenThread={() => {}} />));
		const who = () => container.querySelector(".spine-rule-aside, .spine-who")?.textContent;
		await draw("shared");
		expect(who()).toBe("Pork Chop to Juno · also sent to Ivy, Huck · wants an answer");
		await draw("alone");
		expect(who()).toBe("Pork Chop to Juno · wants an answer");
		await draw("bare");
		expect(container.querySelector(".spine-rule-aside")).toBeNull();
		expect(container.querySelector(".spine-head + .spine-who")?.textContent).toBe("Pork Chop to Juno · wants an answer");
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

	test("a teammate's row in the rail shows no inbox count, however much is unread", async () => {
		const { Rail } = await import("../src/components/Rail");
		const juno = { persona: { id: "juno", name: "Juno", goal: "" }, session: { personaId: "juno", state: "idle" }, waiting: false, drawing: false, inbox: 3 } as unknown as import("../src/wire").RosterEntry;
		await act(async () =>
			root!.render(
				<Rail entries={[juno]} selectedId={null} seen={{}} connection="open" onSelect={() => {}} onEdit={() => {}} onDelete={() => {}} onPin={() => {}} onLead={() => {}} onAddDesk={() => {}} onNew={() => {}} settings={null} search={null} deskKey={null} />,
			),
		);
		const row = container.querySelector("[data-teammate-row]")!;
		expect(row.getAttribute("aria-label")).toBe("Juno");
		expect(row.textContent).not.toContain("3");
	});

	test("a bubble posted from a turn one item woke has one quiet line over it, whose message and its title, that opens the item", async () => {
		const { Transcript } = await import("../src/components/Transcript");
		const woke = item("w", { from: "huck", title: "Rerun the Linux build", state: "closed", outcome: "noted" });
		const notice = item("n", { from: "ivy", title: "Expired: Changelog wording", notice: "expired", state: "closed", outcome: "noted" });
		spyOn(wire, "subscribe").mockImplementation(((target: { inbox?: string }, handlers: { snapshot(items: unknown): void }) => {
			if (target.inbox === "juno") handlers.snapshot(view({ closed: [woke, notice] }));
			return () => {};
		}) as typeof wire.subscribe);
		const opened: string[] = [];
		const events: TranscriptEvent[] = [
			{ kind: "agent", id: "p1", ts: NOW - 50_000, text: "The Linux build is green again.", fromItem: "w" },
			{ kind: "turn", id: "e1", ts: NOW - 49_000, stopReason: "end_turn", quiet: true },
			{ kind: "agent", id: "p2", ts: NOW - 40_000, text: "Ivy never picked up the changelog wording.", fromItem: "n" },
			{ kind: "turn", id: "e2", ts: NOW - 39_000, stopReason: "end_turn", quiet: true },
			{ kind: "user", id: "u1", ts: NOW - 30_000, text: "Thanks" },
			{ kind: "agent", id: "a1", ts: NOW - 20_000, text: "Any time." },
		];
		await act(async () =>
			root!.render(<Transcript personaId="juno" name="Juno" people={people} events={events} streaming={[]} live={false} focus={null} onOpenInboxItem={(id) => opened.push(id)} />),
		);
		const rows = [...container.querySelectorAll<HTMLButtonElement>(".link-row[data-kind=inbox-from]")];
		expect(rows.map((row) => row.querySelector(".link-row-words")?.textContent)).toEqual(["From Huck's message ·Rerun the Linux build", "From Hotline's notice ·Expired: Changelog wording"]);
		// Over its own bubble, and over no other.
		expect(rows[0]?.closest(".said-group")?.textContent).toContain("The Linux build is green again.");
		expect(container.textContent).toContain("Any time.");
		expect(container.querySelectorAll(".said-group .link-row[data-kind=inbox-from]")).toHaveLength(2);
		await press(rows[0]!);
		expect(opened).toEqual(["w"]);
	});

	test("an item taken says Picked up in its state line, and its sender waiting after it", async () => {
		const { InboxItemPane } = await import("../src/components/InboxPane");
		spyOn(wire, "command").mockImplementation((async () => ({ item: item("p", { state: "taken", waiting: true, receipts: [{ state: "new", at: NOW - 60_000 }, { state: "taken", at: NOW - 30_000 }] }), replies: [] })) as unknown as typeof wire.command);
		await act(async () => root!.render(<InboxItemPane itemId="p" people={people} onBack={() => {}} onClose={() => {}} onOpenItem={() => {}} onOpenThread={() => {}} />));
		expect(container.querySelector(".spine-state-word")?.textContent).toBe("Picked up");
		expect(container.querySelector(".spine-state-detail")?.textContent).toBe("· Pork Chop is waiting");
		expect(container.querySelector(".band h2")?.getAttribute("aria-label")).toBe("Inbox, Picked up");
		// The receipts' last line is the item as it stands: open, so a ring.
		expect([...container.querySelectorAll(".inbox-receipts li > .spine-mark")].map((one) => one.getAttribute("data-mark"))).toEqual(["done", "waiting"]);
	});

	test("a held item says it waits for the person to allow it", async () => {
		const { InboxItemPane } = await import("../src/components/InboxPane");
		spyOn(wire, "command").mockImplementation((async () => ({ item: item("h", { held: true, receipts: [] }), replies: [] })) as unknown as typeof wire.command);
		await act(async () => root!.render(<InboxItemPane itemId="h" people={people} onBack={() => {}} onClose={() => {}} onOpenItem={() => {}} onOpenThread={() => {}} />));
		expect(container.textContent).toContain("Waiting for you to allow it.");
		expect(container.textContent).not.toContain("first-contact");
		expect([...container.querySelectorAll(".inbox-receipt-word")].map((one) => one.textContent)).toEqual(["Sent"]);
	});

	test("an item's pane says where it stands beside its title, and a receipt older than today its date as well as its time", async () => {
		const { InboxItemPane } = await import("../src/components/InboxPane");
		const midnight = new Date(NOW);
		midnight.setHours(0, 0, 0, 0);
		const sentAt = midnight.getTime() - 3 * 86_400_000 + 9 * 3_600_000;
		const today = Math.max(midnight.getTime() + 60_000, NOW - 60_000);
		const later = item("l", { ts: sentAt, state: "later", reason: "after the release", receipts: [{ state: "new", at: sentAt }, { state: "later", at: today, reason: "after the release" }] });
		spyOn(wire, "command").mockImplementation((async () => ({ item: later, replies: [] })) as unknown as typeof wire.command);
		await act(async () => root!.render(<InboxItemPane itemId="l" people={people} onBack={() => {}} onClose={() => {}} onOpenItem={() => {}} onOpenThread={() => {}} />));
		expect(container.querySelector(".spine-state-word")?.textContent).toBe("Later");
		const times = [...container.querySelectorAll(".inbox-receipt-at")].map((one) => one.textContent);
		const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
		const day = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(new Date(sentAt));
		expect(times[0]).toContain(day);
		expect(times[0]).toContain(clock.format(new Date(sentAt)));
		expect(times[1]).toBe(clock.format(new Date(today)));
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
		// The thread is the taken item's, not one combined into it.
		const combined = item("c", { state: "closed", outcome: "combined", combinedInto: "b", work: { kind: "side", key: "s-build" } });
		const taken = item("b", { state: "taken", work: { kind: "side", key: "s-build" }, request: "r1" });
		expect(inbox.inboxWork(new Map([["c", combined], ["b", taken], ["n", item("n")]]))).toEqual(new Map([["s-build", "b"]]));
	});

	test("the Threads list and the composer's count list it as a work thread", async () => {
		const { listItems, markOf } = await import("../src/threadList");
		const { liveThreads } = await import("../src/liveThreads");
		const row = { thread: { kind: "side" as const, key: "s-build" }, personaId: "juno", title: "Rerun the Linux build", state: "live" as const, startedAt: NOW - 60_000, updatedAt: NOW, working: true, waiting: false, opener: { personaId: "chop", name: "Pork Chop" } };
		const old = { ...row, thread: { kind: "side" as const, key: "s-old" }, title: "Old handoff", opener: { personaId: "huck", name: "Huck" } };
		const items = listItems({ rows: [row, old], runs: [], jobs: [], asides: [], listedFor: "juno", nameOf, personName: "", now: NOW, inboxWork: new Map([["s-build", "b"]]) });
		expect(items.map((one) => [one.id, one.kind, markOf(one.kind).word])).toEqual([
			["side:s-build", "work", "Work thread"],
			["side:s-old", "handoff", "Handoff"],
		]);
		const juno = { persona: { id: "juno", name: "Juno" }, session: { personaId: "juno", state: "thinking" }, sides: [{ sideId: "s-build", title: "Rerun the Linux build", startedAt: NOW - 60_000, working: true }] } as unknown as import("../src/wire").RosterEntry;
		const live = liveThreads({ listed: [row], roster: [juno], entry: juno, jobs: [], nameOf, now: NOW, inboxWork: new Map([["s-build", "b"]]) });
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
