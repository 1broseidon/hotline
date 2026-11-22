import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { InboxItem, InboxSortedEntry, InboxView, TranscriptEvent } from "../src/generated/contract";
import type { InboxSent } from "../src/inbox";
import type { Block } from "../src/scheduledRuns";
import { installDom } from "./dom";

// task-456: the desk draws the inbox (docs/design/agent-inbox.md, "On the
// desk"). What a turn sent and sorted is in its path, each a quiet row live
// from the teammate's inbox, and the chat draws no Inbox line; the pane
// lists its inbox and opens one item with its receipts. The person reads;
// nothing here sends or sorts, and nothing counts what a teammate has not
// read (George, 2026-11-21: to the person, inbox items are not mail).

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

	test("a row says where its items stand, from the sender's live view, a send to several answers first", () => {
		const group = { title: "Release moves to Thursday", sent: [sent("i1", "juno", "x"), sent("i2", "ivy", "x"), sent("i3", "huck", "x")] as InboxSent[] };
		const known = (items: InboxItem[]) => new Map(items.map((one) => [one.id, one]));
		expect(inbox.sentState(group, new Map())).toBeUndefined();
		expect(inbox.sentState(group, known([item("i1", { state: "seen" }), item("i2", { state: "taken" }), item("i3")]))).toBe("1 picked up · 1 seen · 1 sent");
		expect(inbox.sentState(group, known([item("i1", { state: "closed", outcome: "noted" }), item("i2", { state: "closed", outcome: "noted" }), item("i3", { state: "closed", outcome: "noted" })]))).toBe("3 noted");
		const one = { title: "Rerun the Linux build", sent: [sent("i4", "juno", "x")] as InboxSent[] };
		// The person reads Taken as Picked up; the stored state stays taken.
		expect(inbox.sentState(one, known([item("i4", { state: "taken" })]))).toBe("Picked up");
		expect(inbox.sentState(one, known([item("i4", { state: "later", reason: "after the release" })]))).toBe("Later");
		expect(inbox.sentState(one, known([item("i4", { held: true, receipts: [] })]))).toBe("Awaiting approval");
		expect(inbox.toWords(["juno", "ivy", "huck", "chop"], nameOf)).toBe("Juno, Ivy, Huck +1");
	});

	test("a send's words lead with what came back: done before seen before awaiting approval, whatever order the send named them in", () => {
		const words = inbox.groupWords([item("h", { to: "huck", state: "seen" }), item("j", { to: "juno", held: true, receipts: [] }), item("q", { to: "ivy", state: "closed", outcome: "done" })]);
		expect(words).toBe("1 done · 1 seen · 1 awaiting approval");
		expect(inbox.groupWords([item("a", { state: "closed", outcome: "answered" }), item("b", { state: "closed", outcome: "answered" }), item("c", { state: "noticed" })])).toBe("2 answered · 1 sent");
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

	test("its sorts and sends fold into the turn's steps with its tools and words, and no bubble follows", async () => {
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
		expect(drawn.map((block) => (block.kind === "event" ? block.event.kind : block.kind))).toEqual(["steps", "turn"]);
		const steps = drawn.find((block) => block.kind === "steps");
		expect(steps?.kind === "steps" && steps.items.map((one) => one.kind)).toEqual(["tool", "tool", "inbox_sent", "inbox_sorted", "thought"]);
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
		expect(drawn.map((block) => (block.kind === "event" ? block.event.kind : block.kind))).toEqual(["agent", "steps", "turn"]);
		const steps = drawn.find((block) => block.kind === "steps");
		expect(steps?.kind === "steps" && steps.items.map((one) => one.id)).toEqual(["t1", "inbox-sorted:1", "t2", "w1"]);
	});

	test("sends and sorts in a turn the person started fold into that turn's steps in order, not lines in the chat", async () => {
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
		expect(drawn.map((block) => (block.kind === "event" ? block.event.kind : block.kind))).toEqual(["user", "agent", "steps", "turn"]);
		const steps = drawn.filter((block) => block.kind === "steps");
		expect(steps).toHaveLength(1);
		expect(steps[0]?.kind === "steps" && steps[0].items.map((one) => one.id)).toEqual(["t1", "t2", "inbox-sent:r1", "t3", "inbox-sorted:1", "t4"]);
	});
});

// George, 2026-11-21: the chat never shows a standalone Inbox line, not even
// for a turn with no bubble. What a turn sent or read is on the line under
// its reply; a turn that put nothing in the chat shows nothing.
describe("no Inbox line in the chat", () => {
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
		blocks.flatMap((block) => (block.kind === "event" ? (block.event.kind === "turn" ? [] : [block.event.id]) : [`${block.kind}:${block.id}`]));
	const drawn = async (events: TranscriptEvent[], work: ReadonlySet<string> = new Set()) => {
		const { toBlocks } = await import("../src/transcriptFolds");
		const { withoutUnsaidTurns } = await import("../src/scheduledRuns");
		return withoutUnsaidTurns(groupInbox(toBlocks(events)), work);
	};

	test("quiet turns that read, sent and worked but said nothing draw nothing", async () => {
		const blocks = await drawn([tool("t1", "read_inbox"), sent("r1", "chop", "Re: Test message"), sorted("s1", "i1"), quietEnd("e1"), tool("t2", "read_inbox"), sorted("s2", "i2", "i3"), quietEnd("e2"), tool("t3", "read_inbox"), quietEnd("e3")]);
		expect(shape(blocks)).toEqual([]);
	});

	test("a turn that said something keeps its line, and what it read and sent is on it, wherever the turn started", async () => {
		const blocks = await drawn([
			tool("t0", "read_inbox"),
			sorted("s0", "i0"),
			quietEnd("e0"),
			tool("t1", "read_inbox"),
			sorted("s1", "i1"),
			{ kind: "tool", id: "t2", ts: NOW, toolCallId: "t2", title: "post_to_chat", status: "completed" },
			said("p1", "The crane permit is yours to sign."),
			quietEnd("e1"),
			{ kind: "user", id: "u1", ts: NOW, text: "Ask Huck whether staging is safe to wipe." },
			sent("a1", "huck", "Is staging safe to wipe?"),
			tool("t4", "update_inbox"),
			sorted("s3", "i3"),
			said("a2", "Asked Huck."),
			ended("e3"),
		]);
		expect(shape(blocks)).toEqual(["p1", "steps:t1", "u1", "a2", "steps:inbox-sent:a1"]);
		const lines = blocks.flatMap((block) => (block.kind === "steps" ? [inbox.inboxWords(block.items)] : []));
		expect(lines).toEqual(["Read a message", "Read a message, sent a message"]);
	});

	test("the words count each item a sort moved once and each item sent, and say nothing for a turn that did neither", () => {
		expect(inbox.inboxWords([sorted("s1", "i1", "i2"), sorted("s2", "i2"), sent("r1", "huck", "A"), sent("r2", "ivy", "A")])).toBe("Read 2 messages, sent 2 messages");
		expect(inbox.inboxWords([sent("r1", "huck", "A")])).toBe("Sent a message");
		expect(inbox.inboxWords([tool("t1", "read")])).toBeUndefined();
	});

	test("the work thread an item started is no row of the receiver's chat, running or over; one no item started keeps its row", async () => {
		const work = new Set(["w1"]);
		expect(shape(await drawn([tool("t1", "update_inbox"), sorted("s1", "i1"), link("l1", "w1", "live"), quietEnd("e1")], work))).toEqual([]);
		expect(shape(await drawn([tool("t1", "update_inbox"), sorted("s1", "i1"), link("l1", "w1", "closed"), quietEnd("e1")], work))).toEqual([]);
		expect(shape(await drawn([link("l1", "w2", "live")], work))).toEqual(["l1"]);
	});

	test("a quiet turn still running keeps its block, and one that failed keeps its line", async () => {
		expect(shape(await drawn([tool("t1", "read_inbox"), sorted("s1", "i1")]))).toEqual(["steps:t1"]);
		const failed: TranscriptEvent = { kind: "turn", id: "e1", ts: NOW, stopReason: "error", quiet: true } as TranscriptEvent;
		expect(shape(await drawn([tool("t1", "read_inbox"), failed]))).toEqual(["steps:t1"]);
	});
});

describe("topics", () => {
	/** Goldie's inbox: what she sent and what was sent her, as her view holds it. */
	const goldie = (more: Partial<InboxView> = {}): InboxView => ({ personaId: "chop", open: [], closed: [], sent: [], unread: 0, ...more });

	test("one row per topic, grouped To sort, Picked up, Waiting on others and For later, and the unread count counts only what was not read", () => {
		const ask = item("a1", { from: "chop", to: "juno", topic: "a", sendId: "a", title: "What was the score?", body: "", state: "closed", outcome: "answered" });
		const answer = item("a2", { from: "juno", to: "chop", topic: "a", title: "Re: What was the score?", body: "Devils 7–4.", replyTo: "a1", wants: "nothing", state: "seen" });
		const brief = item("b1", { from: "huck", to: "chop", topic: "b", sendId: "b", state: "taken", work: { kind: "side", key: "s1" } });
		const waiting = item("d1", { from: "chop", to: "ivy", topic: "d", sendId: "d", held: true, receipts: [] });
		const later = item("f1", { from: "juno", to: "chop", topic: "f", sendId: "f", state: "later", reason: "after the numbers are in" });
		const unread = item("e1", { from: "ivy", to: "chop", topic: "e", sendId: "e", state: "noticed" });
		const done = item("g1", { from: "huck", to: "chop", topic: "g", sendId: "g", state: "closed", outcome: "noted" });
		const view = goldie({ open: [unread, later, brief, answer], closed: [done, ask], sent: [waiting, ask], unread: 1 });
		const topics = inbox.topicsOf(view, new Set(["s1"]));
		const place = Object.fromEntries(topics.map((one) => [one.key, one.place]));
		expect(place).toEqual({ a: "sort", b: "picked", d: "waiting", f: "later", e: "sort", g: "closed" });
		// The score is read and not yet sorted: under To sort, and no part of the count.
		const score = topics.find((one) => one.key === "a")!;
		expect(score.items.map((one) => one.id)).toEqual(["a1", "a2"]);
		expect([score.unread, inbox.topicMark(score)]).toEqual([0, "◐"]);
		expect(topics.find((one) => one.key === "e")!.unread).toBe(1);
		expect(inbox.topicLine(score, nameOf)).toEqual({ text: "Juno: Devils 7–4.", warn: false });
		// What Goldie sent that waits on the person is Waiting on others, in amber.
		const held = topics.find((one) => one.key === "d")!;
		expect(inbox.topicLine(held, nameOf)).toEqual({ text: "Ivy · Awaiting approval", warn: true });
		expect(inbox.topicWord(held)).toBe("Awaiting approval");
		expect(inbox.topicLine(topics.find((one) => one.key === "f")!, nameOf).text).toBe("Juno · after the numbers are in");
		expect(topics.find((one) => one.key === "b")!.live).toBe(true);
		expect(inbox.TOPIC_GROUPS.map((group) => group.title)).toEqual(["To sort", "Picked up", "Waiting on others", "For later"]);
	});

	test("an item kept before topics is its own topic, and one send to several is one topic for its sender, its lanes answers first", () => {
		const to = (id: string, who: string, more: Partial<InboxItem> = {}) => item(id, { from: "chop", to: who, topic: "s", sendId: "s", title: "Storage numbers", ...more });
		const view = goldie({
			sent: [to("h", "huck", { state: "seen" }), to("i", "ivy", { state: "closed", outcome: "done" }), to("j", "juno", { held: true, receipts: [] }), item("old", { from: "chop" })],
			closed: [item("r", { from: "ivy", to: "chop", topic: "s", replyTo: "i", title: "Re: Storage numbers", body: "530 MB before, 341 after.", wants: "nothing", state: "closed", outcome: "noted" })],
		});
		const topics = inbox.topicsOf(view);
		expect(topics.find((one) => one.key === "old")?.items.map((one) => one.id)).toEqual(["old"]);
		const send = topics.find((one) => one.key === "s")!;
		expect(send.sends.map((one) => one.to)).toEqual(["huck", "ivy", "juno"]);
		expect(inbox.topicWord(send)).toBe("1 done · 1 seen · 1 awaiting approval");
		expect(inbox.topicLine(send, nameOf).text).toBe("1 done · 1 seen · 1 awaiting approval");
		expect(send.place).toBe("waiting");
		// Each reply stays in the lane of the teammate it came from.
		expect(inbox.lanes(send).map((lane) => [lane.send.to, lane.items.map((one) => one.id)])).toEqual([
			["ivy", ["r"]],
			["huck", []],
			["juno", []],
		]);
		// Search reads every message, and who is in it.
		expect(inbox.topicMatches(send, "341 ivy", nameOf)).toBe(true);
		expect(inbox.topicMatches(send, "windows", nameOf)).toBe(false);
	});

	test("a message is a bubble when it is short and plain, and a letter otherwise", () => {
		const words = (body: string, more: Partial<InboxItem> = {}) => inbox.isShort({ body, ...more });
		expect(words("")).toBe(true);
		expect(words("Devils 7–4.")).toBe(true);
		expect(words("x".repeat(160))).toBe(true);
		expect(words("x".repeat(161))).toBe(false);
		expect(words("Done.\n\nThe table lists Linux and macOS only now.")).toBe(false);
		expect(words("- the storage numbers")).toBe(false);
		expect(words("## Notes")).toBe(false);
		expect(words("1. first")).toBe(false);
		expect(words("Run `make check` first.")).toBe(false);
		expect(words("Here it is.", { files: ["shot.png"] })).toBe(false);
		// A title that is the whole message is said once, as the bubble.
		expect(inbox.bubbleText({ title: "What was the score?", body: "" })).toBe("What was the score?");
		expect(inbox.bubbleText({ title: "Re: x", body: " Devils 7–4. " })).toBe("Devils 7–4.");
	});
});

describe("a work thread's Message", () => {
	test("reads each item's title over its words, never the line the desk hands the teammate; a combined item from someone else says only whom it is from", () => {
		const seeded = [
			"Rerun the Linux build\nFrom Timo, item i5, wants work:\nIt failed overnight.",
			"Check the runner disk\nFrom Timo, item i6, wants work:\n(No message beyond its title.)",
			"Clear the cache\nFrom Quill, item i7, wants work:\nIt is full.",
		].join("\n\n---\n\n");
		expect(inbox.messageWords(seeded)).toBe(
			["Rerun the Linux build  \nIt failed overnight.", "Check the runner disk", "Clear the cache  \nFrom Quill  \nIt is full."].join("\n\n---\n\n"),
		);
	});
});

describe("a teammate woken by its inbox", () => {
	test("a turn the person was not part of ends Worked quietly in the steps pane, never Done; a stopped one is Stopped", async () => {
		const { runEnding } = await import("../src/transcriptFolds");
		const step: TranscriptEvent = { kind: "tool", id: "t1", ts: NOW, toolCallId: "t1", title: "update_inbox", status: "completed" };
		const items = [{ kind: "tool", id: "t1" }] as unknown as import("../src/scheduledRuns").Step[];
		const end = (more: Partial<Extract<TranscriptEvent, { kind: "turn" }>>): TranscriptEvent => ({ kind: "turn", id: "e1", ts: NOW, stopReason: "end_turn", ...more });
		expect(runEnding([step, end({ quiet: true })], items)).toBe("quiet");
		const words = { kind: "thought", id: "words", ts: 2, text: "A private note", said: true } as const;
		expect(runEnding([step, words, end({ quiet: true })], [...items, words])).toBe("quiet-written");
		expect(runEnding([step, end({})], items)).toBe("done");
		expect(runEnding([step, end({ quiet: true, stopReason: "cancelled" })], items)).toBe("stopped");
	});
});

describe("task-456 the pane", () => {
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
		// Held, its history ends waiting on the person.
		expect(inbox.receiptRows(item("w", { ts: sentAt, held: true, receipts: [] })).at(-1)).toEqual({ at: sentAt, word: "Awaiting approval", detail: "needs your OK", warn: true });
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

	test("a sent row reads its state live, answers first, and a send to several opens the whole send in the pane, every teammate's lane", async () => {
		const { InboxRows, InboxSentRow } = await import("../src/components/transcript/InboxLines");
		const opened: string[] = [];
		const members = [sent("i1", "juno", "Release"), sent("i2", "ivy", "Release"), sent("i3", "huck", "Release")] as InboxSent[];
		const one = (id: string, to: string, more: Partial<InboxItem> = {}) => item(id, { to, topic: "s", sendId: "s", title: "Release", ...more });
		const draw = (items: InboxItem[]) =>
			act(async () =>
				root!.render(
					<InboxRows.Provider value={{ work: new Map(), items: new Map(items.map((known) => [known.id, known])), onOpen: (id) => opened.push(id) }}>
						<InboxSentRow title="Release" sent={members} people={people} />
					</InboxRows.Provider>,
				),
			);
		await draw([one("i1", "juno"), one("i2", "ivy"), one("i3", "huck")]);
		const row = () => container.querySelector<HTMLButtonElement>("button.link-row[data-kind=inbox]")!;
		expect(row().textContent).toContain("Release· To Juno, Ivy, Huck · 3 sent");
		const moved = [one("i1", "juno", { state: "seen" }), one("i2", "ivy", { held: true, receipts: [] }), one("i3", "huck", { state: "closed", outcome: "answered" })];
		await draw(moved);
		expect(row().textContent).toContain("To Juno, Ivy, Huck · 1 answered · 1 seen · 1 awaiting approval");
		await press(row());
		expect(opened).toEqual(["i1"]);

		// What the row opens: the topic its first item is in, which is the whole send, Huck's answer first.
		const { InboxTopicPane } = await import("../src/components/InboxTopic");
		const answer = item("r", { from: "huck", to: "chop", topic: "s", replyTo: "i3", title: "Re: Release", body: "Thursday works.", wants: "nothing" });
		const chop = view({ personaId: "chop", sent: moved, open: [answer] });
		const cards: string[] = [];
		await act(async () =>
			root!.render(<InboxTopicPane itemId="i1" view={chop} people={people} working={new Set()} onBack={() => {}} onClose={() => {}} onOpenThread={() => {}} onFollowUp={() => {}} onGoToCard={(to) => cards.push(to)} />),
		);
		expect(container.querySelector(".spine-title")?.textContent).toBe("Release");
		expect(container.querySelector(".spine-state-word")?.textContent).toBe("1 answered · 1 seen · 1 awaiting approval");
		expect([...container.querySelectorAll(".inbox-lane .spine-rule-title")].map((lane) => lane.textContent)).toEqual(["Huck", "Juno", "Ivy"]);
		const lanes = [...container.querySelectorAll(".inbox-lane")];
		expect(lanes[0]?.querySelector(".inbox-bubble")?.textContent).toBe("Thursday works.");
		expect(lanes[1]?.querySelector(".inbox-lane-note")?.textContent).toBe("No reply yet.");
		expect(lanes[2]?.querySelector(".spine-rule-aside")?.getAttribute("data-warn")).toBe("true");
		await press(lanes[2]!.querySelector(".inbox-go")!);
		expect(cards).toEqual(["ivy"]);
	});

	test("Picked up in a path's inbox rows: a sort's row and a sent row say it where the item was taken", async () => {
		const { InboxRows, InboxSentRow, InboxSortedRow } = await import("../src/components/transcript/InboxLines");
		const taken = item("i1", { state: "taken" });
		await act(async () =>
			root!.render(
				<InboxRows.Provider value={{ work: new Map(), items: new Map([["i1", taken]]), onOpen: () => {} }}>
					<InboxSortedRow entry={{ item: "build", title: "Rerun the Linux build", from: "chop", state: "taken" }} people={people} />
					<InboxSentRow title="Release" sent={[sent("i1", "juno", "Release")] as InboxSent[]} people={people} />
				</InboxRows.Provider>,
			),
		);
		expect(container.querySelector("button.link-row[data-kind=inbox]")?.textContent).toContain("To Juno · Picked up");
		expect(container.querySelector(".link-row[data-kind=inbox-sorted]")?.textContent).toContain("From Pork Chop · Picked up");
		expect(container.textContent).not.toContain("Taken");
	});

	test("a sort's row for Hotline's notice credits Hotline, and does nothing where nothing opens it", async () => {
		const { InboxSortedRow } = await import("../src/components/transcript/InboxLines");
		await act(async () => root!.render(<InboxSortedRow entry={{ item: "n", title: "Expired: Changelog wording", from: "huck", state: "closed", outcome: "noted", notice: "expired" }} people={people} />));
		const row = container.querySelector(".link-row")!;
		expect(row.tagName).toBe("P");
		expect(row.textContent).toBe("Expired: Changelog wording· From Hotline · Noted");
	});

	test("the pane lists topics on Open and Closed under their groups, searches every message, and a row opens its topic", async () => {
		const { InboxList } = await import("../src/components/InboxPane");
		const shown = view({
			personaId: "chop",
			open: [
				item("a", { from: "huck", to: "chop", title: "Is the staging database safe to wipe?", waiting: true, topic: "a" }),
				item("s", { from: "ivy", to: "chop", title: "What was the score?", body: "", state: "seen", topic: "s" }),
				item("t", { from: "juno", to: "chop", state: "taken", work: { kind: "side", key: "s1" }, topic: "t" }),
				item("l", { from: "juno", to: "chop", state: "later", reason: "after the build", topic: "l" }),
			],
			closed: [item("c", { from: "huck", to: "chop", state: "closed", outcome: "noted", title: "Nightly deploy is paused", topic: "c" }), item("x", { from: "huck", to: "chop", state: "closed", outcome: "expired", title: "Old nightly artifacts", topic: "x" })],
			sent: [item("w", { from: "chop", to: "ivy", state: "seen", title: "Numbers are in", topic: "w" })],
			unread: 1,
		});
		const tabs: string[] = [];
		const opened: string[] = [];
		const list = (tab: "open" | "closed", query = "") =>
			act(async () =>
				root!.render(<InboxList name="Pork Chop" view={shown} tab={tab} query={query} people={people} working={new Set(["s1"])} onTab={(one) => tabs.push(one)} onQuery={() => {}} onOpenItem={(id) => opened.push(id)} onClose={() => {}} />),
			);
		await list("open");
		expect([...container.querySelectorAll(".list-group")].map((one) => one.textContent)).toEqual(["To sort2", "Picked up1", "Waiting on others1", "For later1"]);
		expect([...container.querySelectorAll("[role=tab]")].map((one) => one.textContent)).toEqual(["Open", "Closed"]);
		// Unread is marked; read and not yet sorted is To sort, and not unread.
		expect(container.querySelector("[data-topic=a]")?.hasAttribute("data-unread")).toBe(true);
		expect(container.querySelector("[data-topic=s]")?.hasAttribute("data-unread")).toBe(false);
		expect(container.querySelector("[data-topic=s] .list-mark")?.textContent).toBe("◐");
		expect(container.querySelector("[data-topic=s] .list-line")?.textContent).toBe("Ivy: What was the score?");
		expect(container.querySelector("[data-topic=w] .list-line")?.textContent).toBe("Ivy · Seen");
		expect(container.querySelector("[data-topic=t] .dock-dot")).not.toBeNull();
		await press(container.querySelector("[data-topic=a]")!);
		expect(opened).toEqual(["a"]);
		await press([...container.querySelectorAll("[role=tab]")].find((one) => one.textContent === "Closed")!);
		expect(tabs).toEqual(["closed"]);
		await list("closed");
		expect(container.querySelector("[data-topic=c] .list-line")?.textContent).toBe("Huck · Noted");
		expect(container.querySelector("[data-topic=c] .list-mark")?.textContent).toBe("✓");
		expect(container.querySelector("[data-topic=x] .list-mark")?.textContent).toBe("◌");
		// A search reads open and closed alike and hides the tabs.
		await list("open", "nightly");
		expect(container.querySelector("[role=tablist]")).toBeNull();
		expect([...container.querySelectorAll(".list-group")].map((one) => one.textContent)).toEqual(["Closed2"]);
		await list("open", "windows");
		expect(container.querySelector(".spine-empty")?.textContent).toBe("Nothing matches.");
		// Nothing in the list sends, sorts or answers.
		expect(container.querySelector("textarea")).toBeNull();
	});

	test("a topic reads as a conversation: the owner on the right, short words as bubbles, long ones as letters, one word under each that opens its history, and the work it started", async () => {
		const { InboxTopicPane } = await import("../src/components/InboxTopic");
		const sentAt = NOW - 300_000;
		const brief = item("b1", {
			from: "huck",
			to: "chop",
			topic: "b",
			ts: sentAt,
			title: "Check the release notes",
			body: "Before the notes go out, check these:\n\n- the storage numbers\n- the links",
			wants: "work",
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
		const question = item("b2", { from: "huck", to: "chop", topic: "b", ts: sentAt + 210_000, replyTo: "b1", title: "Did the Windows row come out too?", body: "", state: "taken", waiting: true, work: { kind: "side", key: "s2" } });
		const reply = item("b3", { from: "chop", to: "huck", topic: "b", ts: sentAt + 220_000, replyTo: "b2", title: "Re: Did the Windows row come out too?", body: "Yes. The table lists Linux and macOS only now.", wants: "nothing", state: "seen" });
		const notice = item("n", { from: "huck", to: "chop", topic: "b", ts: sentAt + 230_000, replyTo: "b3", title: "Expired: Re: Did the Windows row come out too?", wants: "nothing", notice: "expired", state: "closed", outcome: "noted" });
		const shown = view({ personaId: "chop", open: [question], closed: [brief, notice], sent: [reply] });
		const threads: string[] = [];
		const follows: unknown[] = [];
		const commands = spyOn(wire, "command");
		await act(async () =>
			root!.render(
				<InboxTopicPane
					itemId="b1"
					view={shown}
					people={people}
					working={new Set(["s2"])}
					onBack={() => {}}
					onClose={() => {}}
					onOpenThread={(owner, thread) => threads.push(`${owner} ${thread.kind}:${thread.key}`)}
					onFollowUp={(follow) => follows.push(follow)}
					onGoToCard={() => {}}
				/>,
			),
		);
		expect(container.querySelector(".spine-state-word")?.textContent).toBe("Picked up");
		expect(container.querySelector(".spine-state-detail")?.textContent).toBe("· Huck is waiting");
		const messages = [...container.querySelectorAll<HTMLElement>(".inbox-msg")];
		expect(messages.map((one) => [one.dataset["message"], one.dataset["side"], one.querySelector(".inbox-letter") !== null ? "letter" : "bubble"])).toEqual([
			["b1", "left", "letter"],
			["b2", "left", "bubble"],
			["b3", "right", "bubble"],
			["n", "left", "bubble"],
		]);
		expect(messages[3]?.querySelector(".inbox-who")?.textContent).toBe("Hotline");
		// The question's title is the whole of it, said once; the letter's title is the topic's, so it is not said again.
		expect(messages[1]?.querySelector(".inbox-bubble")?.textContent).toBe("Did the Windows row come out too?");
		expect(messages[0]?.querySelector(".inbox-subject")).toBeNull();
		expect([...container.querySelectorAll(".inbox-state")].map((one) => one.textContent)).toEqual(["Done", "Picked up", "Seen", "Noted"]);
		// History is a press away, in place.
		expect(container.querySelector(".inbox-history")).toBeNull();
		await press(container.querySelector(".inbox-state")!);
		expect([...container.querySelectorAll(".inbox-history .inbox-receipt-word")].map((one) => one.textContent)).toEqual(["Sent", "Seen", "Picked up", "Done"]);
		expect(container.querySelector(".inbox-state")?.getAttribute("aria-expanded")).toBe("true");
		// Each work thread a message started is a row under it, live while it runs.
		expect([...container.querySelectorAll(".inbox-work")].map((one) => one.textContent)).toEqual(["≡Work thread · Done", "≡Work thread · Running"]);
		await press(container.querySelectorAll(".inbox-work")[1]!);
		expect(threads).toEqual(["chop side:s2"]);
		// Ask a follow-up targets the newest teammate message, not the desk notice. Nothing is sent from here.
		await press([...container.querySelectorAll("button")].find((one) => one.textContent === "Ask a follow-up")!);
		expect(follows).toEqual([{ aboutItem: "b3", quote: "Check the release notes · Huck" }]);
		expect(commands).not.toHaveBeenCalled();
		expect(container.querySelector("textarea, input")).toBeNull();
	});

	test("Ask a follow-up fills the message box with the topic quoted, and the line sent carries the item it follows up on", async () => {
		const { Composer } = await import("../src/components/Composer");
		const { promptOf } = await import("../src/components/Conversation");
		const cleared: string[] = [];
		await act(async () =>
			root!.render(<Composer personaId="chop" name="Pork Chop" state="ready" replyQuote="What was the score? · Juno" quoteLead="Following up on " onSend={() => {}} onCancel={() => {}} onClearReply={() => cleared.push("x")} />),
		);
		expect(container.querySelector(".quote")?.textContent).toBe("Following up on What was the score? · Juno");
		await press(container.querySelector("[aria-label='Stop following up']")!);
		expect(cleared).toEqual(["x"]);
		expect(promptOf("chop", "Who scored first?", { reply: null, about: { aboutItem: "a2" } }, [])).toEqual({ personaId: "chop", text: "Who scored first?", aboutItem: "a2" });
		expect(promptOf("chop", "Hm?", { reply: { eventId: "e1", text: "x" }, about: null }, [])).toEqual({ personaId: "chop", text: "Hm?", replyTo: "e1" });
	});

	test("the person's follow-up line quotes the topic it is about, and the quote opens it", async () => {
		const { UserBubble } = await import("../src/components/transcript/Bubbles");
		const { InboxRows } = await import("../src/components/transcript/InboxLines");
		const opened: string[] = [];
		const ask = item("a1", { from: "chop", to: "juno", topic: "a", title: "What was the score?" });
		const answer = item("a2", { from: "juno", to: "chop", topic: "a", replyTo: "a1", title: "Re: What was the score?" });
		await act(async () =>
			root!.render(
				<InboxRows.Provider value={{ work: new Map(), items: new Map([ask, answer].map((one) => [one.id, one])), onOpen: (id) => opened.push(id) }}>
					<UserBubble personaId="chop" event={{ kind: "user", id: "u1", ts: NOW, text: "Who scored first?", aboutItem: "a2" }} quote={undefined} people={people} run={{ top: true, bottom: true }} onJump={() => {}} />
				</InboxRows.Provider>,
			),
		);
		expect(container.querySelector(".quote")?.textContent).toBe("JunoWhat was the score?");
		await press(container.querySelector(".quote")!);
		expect(opened).toEqual(["a2"]);
	});

	test("an item a newer Hotline saved is a plain row on the tab it belongs to, saying so, that opens nothing", async () => {
		const { InboxList } = await import("../src/components/InboxPane");
		const newer = (id: string, more: Partial<import("../src/generated/contract").InboxNewerItem>) => ({ id, ts: NOW - 120_000, from: "chop", to: "juno", title: `Title ${id}`, state: "parked", ...more });
		const shown = view({ open: [item("a", { topic: "a" })], unread: 1, newer: [newer("n-open", {}), newer("n-closed", { state: "closed" }), newer("n-sent", { from: "juno", to: "huck" })] });
		const opened: string[] = [];
		const draw = (tab: "open" | "closed") =>
			act(async () => root!.render(<InboxList name="Juno" view={shown} tab={tab} query="" people={people} working={new Set()} onTab={() => {}} onQuery={() => {}} onOpenItem={(id) => opened.push(id)} onClose={() => {}} />));
		await draw("open");
		expect([...container.querySelectorAll("[data-newer]")].map((one) => one.getAttribute("data-newer"))).toEqual(["n-open", "n-sent"]);
		const row = container.querySelector("[data-newer=n-open]")!;
		expect(row.tagName).toBe("DIV");
		expect(row.querySelector(".list-line")?.textContent).toBe("Pork Chop · Saved by a newer version of Hotline. It can't be changed here.");
		expect(container.querySelector("[data-newer=n-sent] .list-line")?.textContent).toBe("To Huck · Saved by a newer version of Hotline. It can't be changed here.");
		// On Open it stands under its own heading on a rule, not under the group before it.
		expect(row.previousElementSibling?.textContent).toBe("From a newer version2");
		await draw("closed");
		expect([...container.querySelectorAll("[data-newer]")].map((one) => one.getAttribute("data-newer"))).toEqual(["n-closed"]);
		expect(container.querySelector(".spine-empty")).toBeNull();
		expect(opened).toEqual([]);
	});

	test("the receiver's conversation draws no row for the work its sort started, while an old handoff keeps its line", async () => {
		const { Transcript } = await import("../src/components/Transcript");
		const juno = view({ open: [item("i-build", { state: "taken", work: { kind: "side", key: "s-build" } })] });
		spyOn(wire, "subscribe").mockImplementation(((target: { inbox?: string }, handlers: { snapshot(items: unknown): void }) => {
			if (target.inbox === "juno") handlers.snapshot(juno);
			return () => {};
		}) as typeof wire.subscribe);
		const events: TranscriptEvent[] = [
			{ kind: "inbox_sorted", id: "sorted", ts: NOW - 2_000, open: 1, unread: 0, entries: [{ item: "i-build", title: "Rerun the Linux build", from: "chop", state: "taken" }] },
			{ kind: "link", id: "link:side:s-build", ts: NOW - 1_000, thread: "s-build", threadKind: "side", personaId: "juno", title: "Rerun the Linux build", state: "live", openerId: "chop", openerName: "Pork Chop" },
			{ kind: "link", id: "link:side:s-old", ts: NOW, thread: "s-old", threadKind: "side", personaId: "juno", title: "Old handoff", state: "live", openerId: "huck", openerName: "Huck" },
		];
		await act(async () =>
			root!.render(<Transcript personaId="juno" name="Juno" people={people} events={events} streaming={[]} live={false} focus={null} workingSides={["s-build", "s-old"]} onOpenThreads={() => {}} onOpenThread={() => {}} />),
		);
		expect(container.querySelector(".link-row[data-inbox]")).toBeNull();
		expect(container.textContent).not.toContain("Rerun the Linux build");
		// The handoff from before the inbox keeps its one line; this work is not in it.
		expect(container.textContent).toContain("Working on a handoff from Huck");
		expect(container.textContent).not.toContain("handoff from Pork Chop");
	});

	test("a teammate's row in the rail shows no inbox count, however much is unread", async () => {
		const { Rail } = await import("../src/components/Rail");
		const juno = { persona: { id: "juno", name: "Juno", goal: "" }, session: { personaId: "juno", state: "idle" }, waiting: false, drawing: false, inbox: 3 } as unknown as import("../src/wire").RosterEntry;
		await act(async () =>
			root!.render(
				<Rail entries={[juno]} selectedId={null} seen={{}} connection="open" onSelect={() => {}} onEdit={() => {}} onDelete={() => {}} onPin={() => {}} onLead={() => {}} onArrange={() => {}} onAddDesk={() => {}} onNew={() => {}} settings={null} search={null} deskKey={null} />,
			),
		);
		const row = container.querySelector("[data-teammate-row]")!;
		expect(row.getAttribute("aria-label")).toBe("Juno");
		expect(row.textContent).not.toContain("3");
	});

	test("a bubble posted from a turn one item woke has no line over it saying whose message it was", async () => {
		const { Transcript } = await import("../src/components/Transcript");
		const woke = item("w", { from: "huck", title: "Rerun the Linux build", state: "closed", outcome: "noted" });
		const notice = item("n", { from: "ivy", title: "Expired: Changelog wording", notice: "expired", state: "closed", outcome: "noted" });
		spyOn(wire, "subscribe").mockImplementation(((target: { inbox?: string }, handlers: { snapshot(items: unknown): void }) => {
			if (target.inbox === "juno") handlers.snapshot(view({ closed: [woke, notice] }));
			return () => {};
		}) as typeof wire.subscribe);
		const events: TranscriptEvent[] = [
			{ kind: "agent", id: "p1", ts: NOW - 50_000, text: "The Linux build is green again.", fromItem: "w" },
			{ kind: "turn", id: "e1", ts: NOW - 49_000, stopReason: "end_turn", quiet: true },
			{ kind: "agent", id: "p2", ts: NOW - 40_000, text: "Ivy never picked up the changelog wording.", fromItem: "n" },
			{ kind: "turn", id: "e2", ts: NOW - 39_000, stopReason: "end_turn", quiet: true },
		];
		await act(async () => root!.render(<Transcript personaId="juno" name="Juno" people={people} events={events} streaming={[]} live={false} focus={null} onOpenInboxItem={() => {}} />));
		expect(container.textContent).toContain("The Linux build is green again.");
		expect(container.textContent).toContain("Ivy never picked up the changelog wording.");
		expect(container.querySelectorAll(".link-row[data-kind=inbox-from]")).toHaveLength(0);
		expect(container.textContent).not.toContain("'s message");
		expect(container.textContent).not.toContain("Hotline's notice");
	});

	test("a message held for the person says Awaiting approval in amber wherever it stands, with the way to its card", async () => {
		const { InboxTopicPane } = await import("../src/components/InboxTopic");
		const held = item("h", { from: "chop", to: "juno", topic: "h", title: "Rotate the staging keys", body: "The keys are 90 days old.", held: true, receipts: [] });
		const cards: string[] = [];
		await act(async () =>
			root!.render(<InboxTopicPane itemId="h" view={view({ personaId: "chop", sent: [held] })} people={people} working={new Set()} onBack={() => {}} onClose={() => {}} onOpenThread={() => {}} onFollowUp={() => {}} onGoToCard={(to) => cards.push(to)} />),
		);
		expect(container.querySelector(".spine-state-word")?.textContent).toBe("Awaiting approval");
		expect(container.querySelector(".spine-state")?.hasAttribute("data-you")).toBe(true);
		expect(container.querySelector(".inbox-state")?.getAttribute("data-warn")).toBe("true");
		await press(container.querySelector(".inbox-go")!);
		expect(cards).toEqual(["juno"]);
		expect(container.textContent).not.toContain("first-contact");
	});

	test("a topic the view no longer holds is read by id, with its replies, and a time older than today has its date", async () => {
		const { InboxTopicPane } = await import("../src/components/InboxTopic");
		const midnight = new Date(NOW);
		midnight.setHours(0, 0, 0, 0);
		const sentAt = midnight.getTime() - 3 * 86_400_000 + 9 * 3_600_000;
		const old = item("o", { from: "huck", to: "juno", ts: sentAt, title: "Rerun the Linux build", body: "", state: "closed", outcome: "answered", receipts: [{ state: "closed", at: sentAt, outcome: "answered" }] });
		const reply = item("r", { from: "juno", to: "huck", ts: sentAt + 60_000, replyTo: "o", title: "Re: Rerun the Linux build", body: "Green again.", wants: "nothing", receipts: [] });
		spyOn(wire, "command").mockImplementation((async () => ({ item: old, replies: [reply] })) as unknown as typeof wire.command);
		await act(async () => root!.render(<InboxTopicPane itemId="o" view={view()} people={people} working={new Set()} onBack={() => {}} onClose={() => {}} onOpenThread={() => {}} onFollowUp={() => {}} onGoToCard={() => {}} />));
		expect([...container.querySelectorAll(".inbox-bubble")].map((one) => one.textContent)).toEqual(["Rerun the Linux build", "Green again."]);
		const day = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(new Date(sentAt));
		expect(container.querySelector(".inbox-meta")?.textContent).toContain(day);
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
	test("the teammate's menu has Inbox beside Threads, with no count of what it has not read", async () => {
		const { teammateActions } = await import("../src/components/Conversation");
		const actions = teammateActions({ inspectorOpen: false, chapterBusy: false, unseen: 0, onToggleInspector: () => {}, onStartChapter: () => {}, onOpenThreadList: () => {}, onOpenInbox: () => {}, onDelete: () => {} });
		const ids = actions.flatMap((one) => (one.kind === "item" ? [one.id] : []));
		expect(ids.indexOf("inbox")).toBe(ids.indexOf("side-list") + 1);
		expect(actions.find((one) => one.kind === "item" && one.id === "inbox")).toEqual({ kind: "item", id: "inbox", text: "Inbox", onSelect: expect.any(Function) });
	});
});


describe("sends in a turn the person reads fold into that turn's Steps (goldie-silent-sends.jsonl)", () => {
	const loadRealTape = async (): Promise<TranscriptEvent[]> => {
		const { readFileSync } = await import("node:fs");
		const raw = readFileSync(new URL("./fixtures/goldie-silent-sends.jsonl", import.meta.url), "utf8");
		return raw
			.trim()
			.split("\n")
			.map((line) => {
				const row = JSON.parse(line) as TranscriptEvent & { output?: unknown };
				// The trimmed tape fixture omits title/toolCallId on redacted tools and replaces output with "(trimmed)".
				if (row.kind === "tool") {
					const { output, ...rest } = row;
					return {
						...rest,
						title: typeof row.title === "string" ? row.title : (row.toolKind ?? ""),
						toolCallId: typeof row.toolCallId === "string" ? row.toolCallId : row.id,
						...(Array.isArray(output) ? { output } : {}),
					} as TranscriptEvent;
				}
				return row as TranscriptEvent;
			});
	};

	const realPeople = new Map([
		["goldie", { name: "Goldie" }],
		["9f959998-3eba-4076-8038-0ecc5a86ca4c", { name: "Huck" }],
		["1e615299-93bd-4441-ab14-76f05369e509", { name: "Quill" }],
		["cb9e5e24-98df-44be-82a8-59bbd8c865c6", { name: "Timo" }],
		["c3a97ea8-dde4-4367-8f55-6be38db28b0c", { name: "Mabel" }],
	]);

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

	test("23:14, 23:27 sends and 23:31, 23:38 answers turns fold into their turns' Steps, whose line says what they sent and read", async () => {
		const events = await loadRealTape();
		const { toBlocks } = await import("../src/transcriptFolds");
		const { withoutUnsaidTurns } = await import("../src/scheduledRuns");
		const blocks = withoutUnsaidTurns(groupInbox(toBlocks(events)), new Set());

		// No chat-level inbox rows anywhere in the built blocks.
		expect(blocks.some((block) => block.kind === "inbox" || (block.kind === "event" && (block.event.kind === "inbox_sent" || block.event.kind === "inbox_sorted")))).toBe(false);

		// 23:14 turn: four sends inside its Steps, none at chat level.
		const turn2314 = blocks.find((block) => block.kind === "steps" && block.id === "tool:5e3e4b05-1676-43cf-bd86-987792c54e54");
		expect(turn2314?.kind).toBe("steps");
		if (turn2314?.kind !== "steps") throw new Error("missing 23:14 steps");
		expect(turn2314.items.filter((one) => one.kind === "inbox_sent")).toHaveLength(4);

		// 23:27 turn: one send inside its Steps (Huck's bean research), none at chat level.
		const turn2327 = blocks.find((block) => block.kind === "steps" && block.id === "inbox-sent:78b2069d-8d87-44ab-82ef-04436503f882");
		expect(turn2327?.kind).toBe("steps");
		if (turn2327?.kind !== "steps") throw new Error("missing 23:27 steps");
		expect(turn2327.items.filter((one) => one.kind === "inbox_sent")).toHaveLength(1);

		// 23:31 answers turn: under the 23:31:53 bubble, its sort and outcome fold into its Steps in tape order.
		const agent2331At = blocks.findIndex((block) => block.kind === "event" && block.event.id === "02abc441-f28c-445c-a2e7-42b3e3785553");
		expect(agent2331At).toBeGreaterThan(0);
		const steps2331 = blocks[agent2331At + 1];
		expect(steps2331?.kind).toBe("steps");
		if (steps2331?.kind !== "steps") throw new Error("missing 23:31 steps");
		expect(steps2331.items.map((one) => (one.kind === "tool" ? `tool:${one.title}` : one.kind))).toEqual([
			"tool:read_inbox",
			"inbox_sorted",
			"tool:update_inbox",
			"outcome",
		]);

		// 23:38 answers turn: under the 23:39:14 table bubble, its sort and outcome fold into its Steps in tape order.
		const agent2338At = blocks.findIndex((block) => block.kind === "event" && block.event.id === "401b4027-11cb-4c57-9ce0-58fa8e88941d");
		expect(agent2338At).toBeGreaterThan(0);
		const steps2338 = blocks[agent2338At + 1];
		expect(steps2338?.kind).toBe("steps");
		if (steps2338?.kind !== "steps") throw new Error("missing 23:38 steps");
		expect(steps2338.items.map((one) => (one.kind === "tool" ? `tool:${one.title}` : one.kind))).toEqual([
			"tool:read_inbox",
			"tool:read",
			"tool:computer__state",
			"tool:computer__state",
			"inbox_sorted",
			"tool:update_inbox",
			"tool:computer__browser",
			"tool:computer__browser",
			"outcome",
		]);

		// Render the real tape in Transcript (in-place Steps, no pane) and verify DOM.
		const { Transcript } = await import("../src/components/Transcript");
		const opened: string[] = [];
		await act(async () =>
			root!.render(<Transcript personaId="goldie" name="Goldie" people={realPeople} events={events} streaming={[]} live={false} focus={null} onOpenInboxItem={(id) => opened.push(id)} />),
		);

		// No inbox row and no Inbox line anywhere at chat level before opening any Steps.
		expect(container.querySelectorAll('[data-kind="inbox"], [data-kind="inbox-sorted"]')).toHaveLength(0);
		expect([...container.querySelectorAll(".steps-caption")].some((caption) => /^Inbox\b/.test(caption.textContent ?? ""))).toBe(false);

		// 23:27 turn: Steps caption is drawn with its time; opened, holds one sent row for Huck.
		const row2327 = container.querySelector<HTMLElement>('[data-event-id="inbox-sent:78b2069d-8d87-44ab-82ef-04436503f882"]')!;
		const caption2327 = row2327.querySelector<HTMLButtonElement>(".steps-caption")!;
		expect(caption2327).not.toBeNull();
		expect(caption2327.querySelector(".steps-caption-took")?.textContent).toBe("14s");
		expect(caption2327.querySelector("[data-kind=inbox-words]")?.textContent).toBe("Sent a message");
		await press(caption2327);
		const sent2327 = [...row2327.querySelectorAll(".steps [data-kind=inbox]")];
		expect(sent2327).toHaveLength(1);
		expect(sent2327[0]?.getAttribute("data-quiet")).toBe("true");
		expect(sent2327[0]?.textContent).toContain("Research bean recommendations for the Ninja AutoBarista Pro");
		expect(sent2327[0]?.textContent).toContain("To Huck");
		await press(sent2327[0]!);
		expect(opened).toEqual(["78b2069d-8d87-44ab-82ef-04436503f882"]);

		// 23:14 turn: opened, holds four sends inside its Steps, none at chat level.
		const row2314 = container.querySelector<HTMLElement>('[data-event-id="tool:5e3e4b05-1676-43cf-bd86-987792c54e54"]')!;
		await press(row2314.querySelector<HTMLButtonElement>(".steps-caption")!);
		const sent2314 = [...row2314.querySelectorAll(".steps [data-kind=inbox]")];
		expect(sent2314).toHaveLength(4);
		expect(sent2314.map((row) => row.textContent)).toEqual([
			expect.stringContaining("To Huck"),
			expect.stringContaining("To Quill"),
			expect.stringContaining("To Timo"),
			expect.stringContaining("To Mabel"),
		]);

		// 23:31 answers turn: Steps under the 23:31:53 bubble, opened, holds one sorted row for Huck's reply (b55617ef).
		const row2331 = container.querySelector<HTMLElement>('[data-event-id="tool:01d7a62e-e180-4f54-9176-22551517aede"]')!;
		await press(row2331.querySelector<HTMLButtonElement>(".steps-caption")!);
		const sorted2331 = [...row2331.querySelectorAll(".steps [data-kind=inbox-sorted]")];
		expect(sorted2331).toHaveLength(1);
		expect(sorted2331[0]?.getAttribute("data-inbox-item")).toBe("b55617ef-c277-4ac5-8154-6bd07e32b2b2");
		expect(sorted2331[0]?.textContent).toContain("Re: Research bean recommendations for the Ninja AutoBarista Pro");
		expect(sorted2331[0]?.textContent).toContain("From Huck");

		// 23:38 answers turn: Steps under the 23:39:14 table bubble, opened, holds one sorted row for Timo's reply (1c298e7e).
		const row2338 = container.querySelector<HTMLElement>('[data-event-id="tool:03506371-5ced-41d7-8329-7ffdcf7cb842"]')!;
		await press(row2338.querySelector<HTMLButtonElement>(".steps-caption")!);
		const sorted2338 = [...row2338.querySelectorAll(".steps [data-kind=inbox-sorted]")];
		expect(sorted2338).toHaveLength(1);
		expect(sorted2338[0]?.getAttribute("data-inbox-item")).toBe("1c298e7e-0b67-48eb-a821-3c47ffae99b8");
		expect(sorted2338[0]?.textContent).toContain("Here are the New Jersey Devils’ next four games after Nov. 20, 2026 (all times…");
		expect(sorted2338[0]?.textContent).toContain("From Timo");
	});

	test("a bubble-posting answers turn with a stamped failed or partial outcome folds it into Steps and shows it on the caption", async () => {
		const { Transcript } = await import("../src/components/Transcript");
		const events: TranscriptEvent[] = [
			{ kind: "tool", id: "t1", ts: NOW - 5_000, endedTs: NOW - 4_000, toolCallId: "t1", title: "read_inbox", status: "completed" },
			{
				kind: "inbox_sorted",
				id: "s1",
				ts: NOW - 4_000,
				open: 0,
				unread: 0,
				entries: [{ item: "i1", title: "Re: Check the schedule", from: "cb9e5e24-98df-44be-82a8-59bbd8c865c6", state: "closed", outcome: "noted" }],
			},
			{ kind: "tool", id: "t2", ts: NOW - 3_000, endedTs: NOW - 2_000, toolCallId: "t2", title: "update_inbox", status: "completed" },
			{ kind: "outcome", id: "o1", ts: NOW - 2_000, type: "made", state: "partial", message: "Confirmed three games; fourth needs manual check", key: "o1", fromItem: "i1" },
			{ kind: "agent", id: "a1", ts: NOW - 1_000, text: "Timo sent the next four games; I confirmed three of them." },
			{ kind: "turn", id: "e1", ts: NOW - 1_000, stopReason: "end_turn" },
		];
		await act(async () =>
			root!.render(<Transcript personaId="goldie" name="Goldie" people={realPeople} events={events} streaming={[]} live={false} focus={null} onOpenInboxItem={() => {}} />),
		);
		const stepsRow = container.querySelector<HTMLElement>('[data-event-id="t1"]')!;
		const captionOutcome = stepsRow.querySelector<HTMLElement>(".steps-caption .outcome");
		expect(captionOutcome?.getAttribute("data-mark")).toBe("partial");
		expect(captionOutcome?.textContent).toBe("Confirmed three games; fourth needs manual check");
		await press(stepsRow.querySelector<HTMLButtonElement>(".steps-caption")!);
		expect(stepsRow.querySelectorAll(".steps [data-kind=inbox-sorted]")).toHaveLength(1);
		expect(stepsRow.querySelectorAll(".steps .step-outcome")).toHaveLength(1);
	});
});
