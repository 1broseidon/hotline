import type { Attachment, TranscriptEvent } from "./generated/contract";
import { bubbleId, pacedLive } from "./pacing";
import { wholeBubbles } from "./reveal";
import type { Block, Step } from "./scheduledRuns";
import type { Streaming } from "./tape";

/**
 * Whose bubble a block is. Anything else on screen — a card, a notice, a
 * chapter line — is `other`, and breaks a run; the machinery between two
 * messages and a turn that draws nothing are `null`, and do not.
 */
export type Side = "me" | "them" | "other" | null;

/** Whether the bubble continues a run from the same side above, and below. */
export type Run = { top: boolean; bottom: boolean };

export function sideOf(block: Block): Side {
	if (block.kind === "scheduled" || block.kind === "calls") return "other";
	if (block.kind !== "event") return null;
	const event = block.event;
	if (event.kind === "user") return event.scheduled === undefined ? "me" : "other";
	if (event.kind === "agent") return "them";
	if (event.kind === "turn") return event.stopReason === "end_turn" ? null : "other";
	return "other";
}

/**
 * Fold runs of thoughts and tools into one block; a streaming thought joins
 * the tail. A reply being written shows each bubble once it is whole, never a
 * bubble that will still grow, cut the way the desk will write it and under
 * the ids it will use, so the written lines replace them in place.
 */
/** The written events as rows: runs of thoughts and tool calls fold into one block of steps. */
export function toBlocks(events: TranscriptEvent[]): Block[] {
	const blocks: Block[] = [];
	for (const event of events) {
		if (event.kind === "computer_pull") continue;
		if (event.kind === "thought" || event.kind === "tool") {
			const tail = blocks[blocks.length - 1];
			if (tail?.kind === "steps") tail.items.push(event);
			else blocks.push({ kind: "steps", id: event.id, ts: event.ts, items: [event] });
		} else {
			blocks.push({ kind: "event", event });
		}
	}
	return blocks;
}

/**
 * The written rows, then what is streaming after them. The written blocks are
 * shared with the last render, so their rows are skipped; a streamed thought
 * joining the last block of steps gets a copy of that block, never an edit.
 */
export function withStreaming(written: Block[], streaming: Streaming[]): Block[] {
	if (streaming.length === 0) return written;
	const blocks = written.slice();
	for (const one of streaming) {
		if (one.kind === "agent") {
			const base = one.bubbleOf?.base ?? one.messageId;
			const start = one.bubbleOf?.index ?? 0;
			pacedLive(wholeBubbles(one.text)).forEach((text, index) => {
				blocks.push({
					kind: "event",
					live: true,
					event: { kind: "agent", id: bubbleId(base, start + index), ts: one.ts, text },
				});
			});
			continue;
		}
		const thought: Step = { kind: "thought", id: one.messageId, ts: one.ts, text: one.text };
		const tail = blocks[blocks.length - 1];
		if (tail?.kind === "steps") blocks[blocks.length - 1] = { ...tail, items: [...tail.items, thought] };
		else blocks.push({ kind: "steps", id: thought.id, ts: thought.ts, items: [thought] });
	}
	return blocks;
}

/**
 * The runs of steps in a tape, in order, each under the id its caption
 * opens it by. What is streaming joins the last run, as it does on screen.
 */
export function stepRuns(events: TranscriptEvent[], streaming: Streaming[]): { id: string; items: Step[] }[] {
	return withStreaming(toBlocks(events), streaming).flatMap((block) => (block.kind === "steps" ? [{ id: block.id, items: block.items }] : []));
}

/**
 * How a run of steps ended: still going, or stopped short by a turn that
 * did not end in its own time. Read from the first turn line after it.
 */
export function runEnding(events: TranscriptEvent[], items: Step[]): "done" | "stopped" {
	const last = items[items.length - 1];
	if (last === undefined) return "done";
	const at = events.findIndex((event) => event.id === last.id);
	if (at < 0) return "done";
	for (const event of events.slice(at + 1)) {
		if (event.kind === "turn") return event.stopReason === "end_turn" ? "done" : "stopped";
	}
	return "done";
}

/** "12 steps", "12 steps · one failed": the caption, and the work pane's line. */
export function stepsSummary(items: Step[]): string {
	const failed = items.some((one) => one.kind === "tool" && one.status === "failed");
	return `${items.length} ${items.length === 1 ? "step" : "steps"}${failed ? " · one failed" : ""}`;
}

/** Only an operator message from this turn can be offered again. A schedule,
 * peer delivery, or a later message is not a failed draft to replay. */
export function retryForNotice(events: TranscriptEvent[], noticeId: string,
    refill: (message: Extract<TranscriptEvent, { kind: "user" }>) => void): (() => void) | undefined {
    const index = events.findIndex((event) => event.id === noticeId);
    if (index < 0 || events.slice(index + 1).some((event) => ["user", "delivery", "chapter"].includes(event.kind))) return undefined;
    for (let at = index - 1; at >= 0; at--) {
        const event = events[at]!;
        if (event.kind === "user") return event.scheduled === undefined ? () => refill(event) : undefined;
        if (["delivery", "chapter", "turn"].includes(event.kind)) return undefined;
    }
    return undefined;
}

type DeliveryEvent = Extract<TranscriptEvent, { kind: "delivery" }>;

/**
 * A colleague's delivery, in the colleague's voice: who, and the start of
 * what they said. The answer is what happened, so the line quotes it rather
 * than the question it answers. `null` for your own answer to a card: the
 * card already says it.
 */
export function deliveryLine(event: DeliveryEvent): { name: string; said: string } | null {
	const cause = event.cause;
	if (cause.kind === "answer") return null;
	if (cause.kind === "peer" && cause.status === "failed")
		return { name: cause.name, said: cause.about ? `didn't answer · ${cause.about}` : "didn't answer" };
	const said = plain(firstLine(withoutOpener(event.text))) || cause.about;
	if (cause.kind !== "handoff") return { name: cause.name, said };
	// A handoff still behind the teammate's current turn says so; once taken up it is just what was handed.
	return { name: cause.name, said: `handed you: ${said}${event.receipt === "sent" ? " · queued" : ""}` };
}

/** A first line that only acknowledges the work, before what came of it. */
const OPENER = /^(on it|got it|sure( thing)?|ok(ay)?|will do|alright|sounds good|here you go|here it is)[.!,:…]*$/i;

/** What a colleague said, without an opener that says nothing when there is more after it. */
function withoutOpener(text: string): string {
	const lines = text.trim().split("\n");
	const rest = lines.slice(1).join("\n").trim();
	return OPENER.test(lines[0]!.trim()) && rest !== "" ? rest : text;
}

/** A quoted line reads as words, not as the markdown it was written in. */
function plain(line: string): string {
	return line.replace(/[`*_]+/g, "");
}

type PeerEvent = Extract<TranscriptEvent, { kind: "peer" }>;

/**
 * The marker's line, in plain words: who, whether it is still going, and how
 * much was said. Once its answer is delivered the marker is dropped (see
 * `superseded`), so this mostly reads while a reply is awaited, or on the
 * side that was asked.
 */
export function peerLine(event: PeerEvent): string {
	const who = event.seat === "client" ? `${event.withName} (outside the room)` : event.withName;
	const what =
		event.status === "failed"
			? `${who} didn't answer`
			: event.role === "caller"
				? event.status === "done"
					? `Talked with ${who}`
					: `Waiting on ${who}`
				: `${who} asked`;
	return event.exchanges > 1 ? `${what} · ${event.exchanges} messages` : what;
}

/** Markers a later delivery already stands for, in the same conversation. */
export function superseded(events: readonly TranscriptEvent[]): Set<string> {
	const answered = new Set<string>();
	const hidden = new Set<string>();
	for (let at = events.length - 1; at >= 0; at--) {
		const event = events[at]!;
		if (event.kind === "delivery" && event.cause.kind !== "answer") answered.add(event.cause.threadKey);
		else if (event.kind === "peer" && event.status !== "waiting" && answered.has(event.threadKey)) hidden.add(event.id);
	}
	return hidden;
}

/** Whether a delivery says something did not come back. */
export function deliveryMissed(event: DeliveryEvent): boolean {
	return event.cause.kind === "peer" && event.cause.status === "failed";
}

/**
 * Why a running turn began, when the last thing before it was a delivery
 * rather than a word from the person: answering a colleague or picking up
 * an answer that arrived while the teammate was away. A person's message
 * or a completed turn ends that cause, so a later turn cannot inherit it.
 */
export function turnCauseLine(events: TranscriptEvent[]): string | null {
	for (let index = events.length - 1; index >= 0; index--) {
		const event = events[index]!;
		if (event.kind === "user" || event.kind === "turn") return null;
		if (event.kind === "delivery") {
			// A queued delivery must not rename the person's current turn.
			if (event.receipt !== "read") continue;
			const cause = event.cause;
			return cause.kind === "answer" ? "Picking up your answer" : `Answering ${cause.name}`;
		}
	}
	return null;
}

/** One line quoted the way the phone quotes it: whitespace folded, 140 characters. */
export function reactionQuote(text: string): string {
	return `> ${text
		.split("\n")
		.map((line) => line.trim())
		.filter(Boolean)
		.join(" ")
		.slice(0, 140)}`;
}

const EMOJI_ONLY = /^(?:\p{Extended_Pictographic}|\p{Emoji_Modifier}|\u200d|\ufe0f)+$/u;

/**
 * Reactions sent as lines of their own — a quoted line and nothing but an
 * emoji, the phone's shape and this window's — folded onto the line they
 * answer, found by its id or else by the quote. The same rule as the phone's
 * `conversationItems`, so both ends draw one tape alike.
 */
export function foldReactions(events: TranscriptEvent[]): { lines: Set<string>; on: Map<string, string[]> } {
	const lines = new Set<string>();
	const on = new Map<string, string[]>();
	events.forEach((event, index) => {
		if (event.kind !== "user" || (event.attachments?.length ?? 0) > 0) return;
		const match = /^(> [^\n]*)\n+([^\n]+)$/.exec(event.text.trim());
		if (!match || !EMOJI_ONLY.test(match[2]!.trim())) return;
		let target = event.replyTo;
		if (target === undefined) {
			for (let i = index - 1; i >= 0; i--) {
				const candidate = events[i]!;
				if (candidate.kind !== "user" && candidate.kind !== "agent") continue;
				const said = candidate.kind === "agent" ? candidate.text || candidate.attachments?.[0]?.name || "" : candidate.text;
				if (reactionQuote(said) === match[1]) {
					target = candidate.id;
					break;
				}
			}
		}
		if (target === undefined) return;
		lines.add(event.id);
		on.set(target, [...(on.get(target) ?? []), match[2]!.trim()]);
	});
	return { lines, on };
}

/** First line of a say, or nothing — a missing or empty original is not a quote. */
export function quotedLine(event: TranscriptEvent): string | undefined {
	if (event.kind !== "user" && event.kind !== "agent") return undefined;
	const line = lineOf(event);
	return line.length > 0 ? line : undefined;
}

/** A say's first line, or the names of its files when it came without words. */
export function lineOf(event: { text: string; attachments?: Attachment[] }): string {
	const line = firstLine(event.text);
	if (line.length > 0) return line;
	return firstLine((event.attachments ?? []).map((file) => file.name).join(", "));
}

function firstLine(text: string): string {
	const line = text.trim().split("\n", 1)[0] ?? "";
	return line.length > 70 ? `${line.slice(0, 70)}…` : line;
}
