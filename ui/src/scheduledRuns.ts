import type { ScheduledRun, TranscriptEvent } from "./generated/contract";

export type Step = Extract<TranscriptEvent, { kind: "thought" | "tool" }>;

/** A prompt a schedule or loop sent, as the operator's line it arrives on. */
export type ScheduledEvent = Extract<TranscriptEvent, { kind: "user" }> & { scheduled: ScheduledRun };

/**
 * Either one event, or a run of the machinery between two messages —
 * thoughts and tool calls — folded into one block so a transcript of forty
 * tool calls still reads as a conversation. A job that fired again and again
 * with nothing between is one block too, oldest run first, sitting where the
 * newest run sits.
 */
export type Block =
	| { kind: "event"; event: Exclude<TranscriptEvent, Step> }
	| { kind: "steps"; id: string; ts: number; items: Step[] }
	| { kind: "scheduled"; id: string; ts: number; jobId: string; name: string; runs: ScheduledEvent[] };

function scheduledOf(block: Block): ScheduledEvent | undefined {
	if (block.kind !== "event" || block.event.kind !== "user" || block.event.scheduled === undefined) return undefined;
	return block.event as ScheduledEvent;
}

/** A turn that ended on its own draws nothing, so it does not come between two runs. */
function drawsNothing(block: Block): boolean {
	return block.kind === "event" && block.event.kind === "turn" && block.event.stopReason === "end_turn";
}

/**
 * Fold two or more scheduled runs of one job into a single block. Give it the
 * blocks that will be drawn: anything drawn between two runs, a message, a
 * card, a line of work, a chapter, another job, breaks the group. Blocks that
 * draw nothing stay where they were and are stepped over.
 */
export function groupScheduled(blocks: Block[]): Block[] {
	const out: Block[] = [];
	let at = 0;
	while (at < blocks.length) {
		const first = scheduledOf(blocks[at]!);
		if (first === undefined) {
			out.push(blocks[at++]!);
			continue;
		}
		const runs = [first];
		const quiet: Block[] = [];
		let pending: Block[] = [];
		let end = at + 1;
		for (; end < blocks.length; end++) {
			const block = blocks[end]!;
			if (drawsNothing(block)) {
				pending.push(block);
				continue;
			}
			const run = scheduledOf(block);
			if (run === undefined || run.scheduled.jobId !== first.scheduled.jobId) break;
			runs.push(run);
			quiet.push(...pending);
			pending = [];
		}
		if (runs.length === 1) {
			out.push(blocks[at++]!);
			continue;
		}
		const newest = runs[runs.length - 1]!;
		out.push(...quiet, {
			kind: "scheduled",
			id: newest.id,
			ts: newest.ts,
			jobId: first.scheduled.jobId,
			name: newest.scheduled.name,
			runs,
		});
		// What draws nothing after the last run belongs to whatever comes next.
		at = end - pending.length;
	}
	return out;
}
