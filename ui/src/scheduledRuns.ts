import type { Attachment, ScheduledRun, TranscriptEvent } from "./generated/contract";

/**
 * A helper (a subagent) or a command left running, as the line its teammate's
 * tape holds for it. Its place in the turn that started it is its place in
 * that turn's path; the conversation has no row for it.
 */
export type RunLink = Extract<TranscriptEvent, { kind: "link" }> & { threadKind: "run" };

export const isRunLink = (event: TranscriptEvent): event is RunLink => event.kind === "link" && event.threadKind === "run";

/**
 * The path of a turn: its thoughts and tool calls, what the teammate said
 * it was on (`activity`, which heads the steps after it) and what it left
 * behind (`outcome`, which the line under the reply is made of), and the
 * helpers and commands it started (`RunLink`).
 */
export type Step = Extract<TranscriptEvent, { kind: "thought" | "tool" | "activity" | "outcome" }> | RunLink;

/** A prompt a schedule or loop sent, as the operator's line it arrives on. */
export type ScheduledEvent = Extract<TranscriptEvent, { kind: "user" }> & { scheduled: ScheduledRun };

/** A finished call, as the conversation's line for it. */
export type CallLink = Extract<TranscriptEvent, { kind: "link" }> & { threadKind: "call" };

type AgentEvent = Extract<TranscriptEvent, { kind: "agent" }>;

/**
 * Either one event, or a turn's machinery — its thoughts and tool calls, and
 * what the teammate said of them — folded into one block under its last bubble, so a transcript of forty tool
 * calls still reads as a conversation. A turn still running has its block
 * last and `open`. A job that fired again and again
 * with nothing between is one block too, oldest run first, sitting where the
 * newest run sits; so are finished calls one after another, and a turn's
 * files sent a message each. A run of a teammate's inbox activity in its
 * own conversation is one block too (`foldInboxRuns`); a send in a turn the
 * person asked for stays its own row, `asked`.
 */
export type Block =
	| { kind: "event"; event: Exclude<TranscriptEvent, Step>; live?: boolean }
	/** `rowSays`: the turn's only work was the inbox, and its sent row says what came of it (`sentRowsSay`). */
	| { kind: "steps"; id: string; ts: number; items: Step[]; open?: boolean; rowSays?: true }
	| { kind: "scheduled"; id: string; ts: number; jobId: string; name: string; runs: ScheduledEvent[] }
	| { kind: "calls"; id: string; ts: number; calls: CallLink[] }
	| { kind: "sent"; id: string; ts: number; messages: AgentEvent[]; files: { eventId: string; index: number; file: Attachment }[] }
	| { kind: "inbox"; id: string; ts: number; title: string; sent: InboxSentEvent[]; asked?: true }
	| { kind: "inboxRun"; id: string; ts: number; members: Block[]; read: number; sent: number };

type InboxSentEvent = Extract<TranscriptEvent, { kind: "inbox_sent" }>;

function scheduledOf(block: Block): ScheduledEvent | undefined {
	if (block.kind !== "event" || block.event.kind !== "user" || block.event.scheduled === undefined) return undefined;
	return block.event as ScheduledEvent;
}

/** A turn that ended on its own draws nothing, so it does not come between two runs. */
function drawsNothing(block: Block): boolean {
	return block.kind === "event" && block.event.kind === "turn" && block.event.stopReason === "end_turn";
}

/**
 * Fold two or more members of a kind with nothing drawn between them into
 * one block that sits where the newest sits. Give it the blocks that will be
 * drawn: anything drawn between two members, a message, a card, a line of
 * work, a chapter, or a member that does not go `together` with the first,
 * breaks the group. Blocks that draw nothing stay where they were and are
 * stepped over.
 */
function groupRuns<T>(blocks: Block[], member: (block: Block) => T | undefined, together: (first: T, next: T) => boolean, fold: (members: T[]) => Block): Block[] {
	const out: Block[] = [];
	let at = 0;
	while (at < blocks.length) {
		const first = member(blocks[at]!);
		if (first === undefined) {
			out.push(blocks[at++]!);
			continue;
		}
		const members = [first];
		const quiet: Block[] = [];
		let pending: Block[] = [];
		let end = at + 1;
		for (; end < blocks.length; end++) {
			const block = blocks[end]!;
			if (drawsNothing(block)) {
				pending.push(block);
				continue;
			}
			const next = member(block);
			if (next === undefined || !together(first, next)) break;
			members.push(next);
			quiet.push(...pending);
			pending = [];
		}
		if (members.length === 1) {
			out.push(blocks[at++]!);
			continue;
		}
		out.push(...quiet, fold(members));
		// What draws nothing after the last member belongs to whatever comes next.
		at = end - pending.length;
	}
	return out;
}

/** Fold two or more scheduled runs of one job into a single block. */
export function groupScheduled(blocks: Block[]): Block[] {
	return groupRuns(
		blocks,
		scheduledOf,
		(first, next) => next.scheduled.jobId === first.scheduled.jobId,
		(runs) => {
			const newest = runs[runs.length - 1]!;
			return { kind: "scheduled", id: newest.id, ts: newest.ts, jobId: runs[0]!.scheduled.jobId, name: newest.scheduled.name, runs };
		},
	);
}

function finishedCall(block: Block): CallLink | undefined {
	if (block.kind !== "event" || block.event.kind !== "link") return undefined;
	const link = block.event;
	return link.threadKind === "call" && link.state === "closed" ? (link as CallLink) : undefined;
}

/**
 * Fold two or more finished calls into one block, oldest first: a morning
 * of trying calls is one line, not a column of them. A call still going
 * stays its own line.
 */
export function groupCalls(blocks: Block[]): Block[] {
	return groupRuns(blocks, finishedCall, () => true, (calls) => {
		const newest = calls[calls.length - 1]!;
		return { kind: "calls", id: newest.id, ts: newest.ts, calls };
	});
}

/** A teammate's message that is only files, written and whole: what `send_file` posts, a file a message. */
function onlyFiles(block: Block): AgentEvent | undefined {
	if (block.kind !== "event" || block.live === true || block.event.kind !== "agent") return undefined;
	const event = block.event;
	return event.text.trim() === "" && event.spoken === undefined && (event.attachments?.length ?? 0) > 0 ? event : undefined;
}

/**
 * Fold two or more messages of only files, from the same place, into one
 * block: a turn that sends five files one at a time is one block of five.
 * Each file keeps the message and the place in it that it is read back by.
 */
export function groupSent(blocks: Block[]): Block[] {
	return groupRuns(
		blocks,
		onlyFiles,
		(first, next) => first.from?.kind === next.from?.kind && first.from?.thread === next.from?.thread,
		(messages) => {
			const newest = messages[messages.length - 1]!;
			const files = messages.flatMap((message) => (message.attachments ?? []).map((file, index) => ({ eventId: message.id, index, file })));
			// Named by its first message, so a file joining the run does not
			// make it a new row (its pictures read again); dated by its newest.
			return { kind: "sent", id: messages[0]!.id, ts: newest.ts, messages, files };
		},
	);
}

function inboxSent(block: Block): InboxSentEvent | undefined {
	return block.kind === "event" && block.event.kind === "inbox_sent" ? block.event : undefined;
}

/**
 * Every item a teammate sent, as one row each `send`: the items one `send`
 * made, to several teammates under one title, stand side by side on the
 * tape, and are one block, named by the first. Anything between two of them,
 * even a turn's end, makes them two rows.
 */
export function groupInbox(blocks: Block[]): Block[] {
	const out: Block[] = [];
	for (const block of blocks) {
		const sent = inboxSent(block);
		if (sent === undefined) {
			out.push(block);
			continue;
		}
		const last = out[out.length - 1];
		if (last?.kind === "inbox" && last.title === sent.title && !last.sent.some((one) => one.to === sent.to)) {
			out[out.length - 1] = { ...last, ts: sent.ts, sent: [...last.sent, sent] };
			continue;
		}
		out.push({ kind: "inbox", id: sent.id, ts: sent.ts, title: sent.title, sent: [sent] });
	}
	return out;
}

/**
 * A turn's part of the blocks, from the line after the last person's line or
 * turn's end to the next: `asked` when the person's own line began it (a
 * scheduled one is no ask), `said` when it put a bubble in the chat, and
 * `sent` when it holds a sent row.
 */
type Turn = { asked: boolean; said: boolean; sent: boolean };

/** The turn each block belongs to, by its index. A person's line or a turn's end starts the next. */
function turnsOf(blocks: Block[]): Turn[] {
	const of: Turn[] = [];
	let turn: Turn = { asked: false, said: false, sent: false };
	for (const block of blocks) {
		const event = block.kind === "event" ? block.event : undefined;
		if (event?.kind === "user" || event?.kind === "turn") {
			turn = { asked: event.kind === "user" && event.scheduled === undefined, said: false, sent: false };
		}
		if (block.kind === "sent" || event?.kind === "agent") turn.said = true;
		if (block.kind === "inbox") turn.sent = true;
		of.push(turn);
	}
	return of;
}

/**
 * Steps whose every tool call was the inbox's (`send`, `read_inbox`,
 * `update_inbox`), with at least one. Saying what it was on or what came of
 * it, a reaction and a post to the chat are no work of their own; a helper or
 * a command started is.
 */
function inboxWorkOnly(items: Step[]): boolean {
	let inbox = false;
	for (const item of items) {
		if (item.kind === "link") return false;
		if (item.kind !== "tool") continue;
		const key = item.said?.key;
		if (key === "colleague") inbox = true;
		else if (key !== "react" && key !== "post") return false;
	}
	return inbox;
}

/**
 * A turn whose only work was the inbox, and that has a sent row in the chat,
 * draws no outcome line: the row says what went and where it stands
 * (`rowSays`, its steps' line its time alone). A turn the person asked for
 * that also put no bubble in the chat draws no time either: the row stands
 * for the turn, and its path is still a turn of the pane's. Only the window
 * draws it so; what the desk keeps and sends, and the phone's line, do not
 * change. An outcome that failed, or only partly happened, keeps the line:
 * a failure is never left out.
 */
export function sentRowsSay(blocks: Block[]): Block[] {
	const turns = turnsOf(blocks);
	return blocks.flatMap((block, at) => {
		const turn = turns[at]!;
		if (block.kind !== "steps" || block.open === true || !turn.sent || !inboxWorkOnly(block.items)) return [block];
		if (block.items.some((item) => item.kind === "outcome" && (item.state === "failed" || item.state === "partial"))) return [block];
		if (turn.asked && !turn.said) return [];
		return [{ ...block, rowSays: true }];
	});
}

/** A blocks' line as its fold reads it: one that joins the run, one that stands beside it, or one that ends it. */
type Place = "member" | "beside" | "end";

/**
 * A teammate's inbox activity in its own conversation, a run at a time, as
 * one quiet line ("Inbox · read 3, sent 1") that opens to its rows in the
 * order they happened: the items it sorted, the items it sent, the work an
 * item started once that work is over, and the time of each turn its inbox
 * woke that said nothing in the chat (a quiet turn's "5.0s"). A run ends at
 * what the person should read: their own line, a bubble the teammate
 * posted, a turn they asked for (whose sent rows stay out of the line as
 * quiet rows of their own, `asked`, being what came of their ask), and any
 * card, notice or other row. What draws nothing, a quiet turn still
 * running, and a work thread an item started while it is not over stand
 * beside the run without ending it: they are drawn under its line, and a
 * work thread joins the line once it is over. The line sits where the run's
 * first row is, and is named and dated by it, so a run that grows keeps its
 * row, open or not, and only its counts move. A run that read and sent nothing (a
 * quiet turn's time alone, a finished work thread alone) is drawn as its
 * rows were. `work` is the work threads inbox items started, by key.
 */
export function foldInboxRuns(blocks: Block[], work: { has(key: string): boolean }): Block[] {
	const out: Block[] = [];
	let run: { block: Block; member: boolean }[] | null = null;
	// The turn going now: the person's line began it or was steered into it, and it posted a bubble.
	let asked = false;
	let posted = false;
	const flush = () => {
		if (run === null) return;
		const members = run.filter((one) => one.member).map((one) => one.block);
		const beside = run.filter((one) => !one.member).map((one) => one.block);
		const read = new Set(members.flatMap((block) => (block.kind === "event" && block.event.kind === "inbox_sorted" ? block.event.entries.map((entry) => entry.item) : []))).size;
		const sent = members.filter((block) => block.kind === "inbox").length;
		if (read + sent === 0) out.push(...run.map((one) => one.block));
		else {
			const first = members[0]!;
			const [id, ts] = first.kind === "event" ? [first.event.id, first.event.ts] : [first.id, first.ts];
			out.push({ kind: "inboxRun", id: `inbox-run:${id}`, ts, members, read, sent }, ...beside);
		}
		run = null;
	};
	blocks.forEach((block, at) => {
		const place = placeIn(block, at);
		if (place === "end") {
			flush();
			out.push(block.kind === "inbox" && asked ? { ...block, asked: true } : block);
		} else if (place === "member") (run ??= []).push({ block, member: true });
		else if (run !== null) run.push({ block, member: false });
		else out.push(block);
		if (block.kind !== "event") return;
		const event = block.event;
		if (event.kind === "user" && event.scheduled === undefined) asked = true;
		else if (event.kind === "agent") posted = true;
		else if (event.kind === "turn") {
			asked = false;
			posted = false;
		}
	});
	flush();
	return out;

	function placeIn(block: Block, at: number): Place {
		if (block.kind === "inbox") return asked ? "end" : "member";
		if (block.kind === "steps") {
			if (block.open === true) return "beside";
			return !posted && endedQuiet(blocks, at) ? "member" : "end";
		}
		if (block.kind !== "event") return "end";
		const event = block.event;
		if (event.kind === "inbox_sorted") return "member";
		if (event.kind === "turn") return event.stopReason === "end_turn" ? "beside" : "end";
		if (event.kind === "link" && event.threadKind === "side" && work.has(event.thread)) return event.state === "closed" ? "member" : "beside";
		return "end";
	}
}

/** Whether the turn a run of steps belongs to ended quiet: the first turn's end or person's line after it says. */
function endedQuiet(blocks: Block[], at: number): boolean {
	for (const block of blocks.slice(at + 1)) {
		if (block.kind !== "event") continue;
		if (block.event.kind === "turn") return block.event.quiet === true;
		if (block.event.kind === "user") return false;
	}
	return false;
}
