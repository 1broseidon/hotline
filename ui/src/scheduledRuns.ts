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
 * files sent a message each.
 */
export type Block =
	| { kind: "event"; event: Exclude<TranscriptEvent, Step>; live?: boolean }
	| { kind: "steps"; id: string; ts: number; items: Step[]; open?: boolean }
	| { kind: "scheduled"; id: string; ts: number; jobId: string; name: string; runs: ScheduledEvent[] }
	| { kind: "calls"; id: string; ts: number; calls: CallLink[] }
	| { kind: "sent"; id: string; ts: number; messages: AgentEvent[]; files: { eventId: string; index: number; file: Attachment }[] };

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
