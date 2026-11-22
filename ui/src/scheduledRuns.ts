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
export type Step = Extract<TranscriptEvent, { kind: "thought" | "tool" | "activity" | "outcome" | "inbox_sent" | "inbox_sorted" }> | RunLink;

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
 * files sent a message each. A turn's sends and sorts fold into its Steps
 * (`toBlocks`), and a turn that said nothing in the chat draws nothing
 * (`withoutUnsaidTurns`).
 */
export type Block =
	| { kind: "event"; event: Exclude<TranscriptEvent, Step> | Extract<TranscriptEvent, { kind: "outcome" | "inbox_sent" | "inbox_sorted" }>; live?: boolean }
	| { kind: "steps"; id: string; ts: number; items: Step[]; open?: boolean; elapsed?: number }
	| { kind: "scheduled"; id: string; ts: number; jobId: string; name: string; runs: ScheduledEvent[] }
	| { kind: "calls"; id: string; ts: number; calls: CallLink[] }
	| { kind: "sent"; id: string; ts: number; messages: AgentEvent[]; files: { eventId: string; index: number; file: Attachment }[] }
	| { kind: "inbox"; id: string; ts: number; title: string; sent: InboxSentEvent[] };

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
 * What the teammate's own conversation draws of a turn that put nothing in
 * the chat: nothing. A turn that ended quiet with no bubble (an inbox wake,
 * a helper coming back) leaves no line, however much it read, sent or did,
 * and the work thread an inbox item started is no row of its own: both are
 * a colleague's business or the machinery's, and the person reads only
 * what a turn said. What it sent or read in a turn that did say something
 * is on the line under that reply (`toBlocks`, `inboxWords`). A turn still
 * running, and one that failed, keep theirs. `work` is the work threads
 * inbox items started, by key.
 */
export function withoutUnsaidTurns(blocks: Block[], work: { has(key: string): boolean }): Block[] {
	let posted = false;
	return blocks.filter((block, at) => {
		if (block.kind === "steps") return block.open === true || posted || !endedQuiet(blocks, at);
		if (block.kind !== "event") return true;
		const event = block.event;
		if (event.kind === "agent") posted = true;
		else if (event.kind === "turn" || event.kind === "user") posted = false;
		return !(event.kind === "link" && event.threadKind === "side" && work.has(event.thread));
	});
}

/** Whether the turn a run of steps belongs to ended quiet on its own: the first turn's end or person's line after it says. */
function endedQuiet(blocks: Block[], at: number): boolean {
	for (const block of blocks.slice(at + 1)) {
		if (block.kind !== "event") continue;
		if (block.event.kind === "turn") return block.event.quiet === true && block.event.stopReason === "end_turn";
		if (block.event.kind === "user") return false;
	}
	return false;
}
