import { useMemo } from "react";
import type { ThreadId, TranscriptEvent } from "../../generated/contract";
import type { Person } from "../../avatars";
import type { Block } from "../../scheduledRuns";
import type { Streaming } from "../../tape";
import { toBlocks, withStreaming } from "../../transcriptFolds";
import { CookieAskCard } from "../transcript/CookieAskCard";
import { CredentialAsk } from "../transcript/CredentialAsk";
import { HumanAction, PasskeyAskCard, Permission } from "../transcript/Asks";
import { NamedSay } from "../transcript/Bubbles";
import { Plan } from "../transcript/Plan";
import { SecretUseAsk } from "../transcript/SecretUseAsk";
import { TrailSteps, useHot } from "../transcript/Trail";
import { LiveStream, TaskCard } from "./Pane";

type Teammate = { id: string; name: string; hash?: string | undefined };
type Delivery = Extract<TranscriptEvent, { kind: "delivery" }>;

/** The delivery that handed the thread over: the request its card shows. */
function handoffRequest(events: readonly TranscriptEvent[]): Delivery | undefined {
	return events.find((event): event is Delivery => event.kind === "delivery" && event.cause.kind === "handoff");
}

/**
 * A handoff's body (docs/design/thread-panes.html, Handoff): the request it
 * was handed, then the receiver's steps and words as they happen, the way a
 * subagent's pane shows its run. The receiver's words go back to the sender,
 * so each says so ("Poe → Elvis"); the person's own lines are theirs, and a
 * card the receiver raises is answered here as anywhere. The band, the
 * composer and the bar above it stay the thread's.
 */
export function HandoffPane({
	thread,
	events,
	streaming,
	live,
	receiver,
	sender,
	people,
}: {
	thread: ThreadId;
	/** The thread's lines, its link left out. */
	events: TranscriptEvent[];
	streaming: Streaming[];
	/** A turn of the receiver's is running. */
	live: boolean;
	receiver: Teammate;
	sender: { personaId: string; name: string };
	people: ReadonlyMap<string, Person>;
}) {
	const request = handoffRequest(events);
	const blocks = useMemo(
		// Each run of steps where it fell between the words, as the receiver works.
		() => withStreaming(toBlocks(events.filter((event) => event !== request), true), streaming),
		[events, request, streaming],
	);
	const hot = useHot();
	const last = blocks.at(-1);
	// Between its steps and its words a running turn is still the receiver's work.
	const thinking = live && !(last?.kind === "steps" && last.open === true) && !(last?.kind === "event" && last.live === true);
	return (
		<>
			{request !== undefined && (
				<TaskCard eyebrow="Request" from={{ personaId: sender.personaId, name: sender.name, hash: people.get(sender.personaId)?.hash }} text={request.text} />
			)}
			<LiveStream following={`${thread.kind}:${thread.key}`} count={blocks.length} content={blocks} live={live}>
				{blocks.map((block) => (
					<HandoffBlock key={blockKey(block)} block={block} thread={thread} receiver={receiver} sender={sender} hot={hot} />
				))}
				{thinking && (
					<p className="pane-thinking">
						<span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
						Thinking
					</p>
				)}
			</LiveStream>
		</>
	);
}

function blockKey(block: Block): string {
	return block.kind === "event" ? block.event.id : block.id;
}

function HandoffBlock({
	block,
	thread,
	receiver,
	sender,
	hot,
}: {
	block: Block;
	thread: ThreadId;
	receiver: Teammate;
	sender: { personaId: string; name: string };
	hot: ReturnType<typeof useHot>;
}) {
	if (block.kind === "steps") return <TrailSteps items={block.items} settled={block.open !== true} hot={hot} />;
	if (block.kind !== "event") return null;
	const event = block.event;
	switch (event.kind) {
		case "agent":
			return <NamedSay name={`${receiver.name} → ${sender.name}`} mine={false} text={event.text} />;
		case "user":
			return <NamedSay name="You" mine text={event.text} />;
		// A follow-up the sender sent into the thread, or a colleague's answer: in their name.
		case "delivery":
			return event.cause.kind === "answer" ? null : <NamedSay name={`${event.cause.name} → ${receiver.name}`} mine={false} text={event.text} />;
		case "turn":
			return event.stopReason === "end_turn" ? null : <p className="instrument mt-1 text-right text-ink-4">{event.stopReason.replace(/_/g, " ")}</p>;
		case "notice":
			return (
				<p className="rule-line rule-line-plain selectable" style={{ color: event.level === "info" ? "var(--ink-3)" : event.level === "warn" ? "var(--warn)" : "var(--danger)" }}>
					{event.text}
				</p>
			);
		case "plan":
			return <Plan entries={event.entries} />;
		case "permission":
			return <Permission thread={thread} event={event} who={receiver} />;
		case "human_action":
			return <HumanAction thread={thread} event={event} who={receiver} />;
		case "credential_ask":
			return <CredentialAsk event={event} teammate={receiver.name} />;
		case "cookie_ask":
			return <CookieAskCard event={event} personaId={receiver.id} teammate={receiver.name} />;
		case "secret_use_ask":
			return <SecretUseAsk event={event} teammate={receiver.name} />;
		case "passkey_ask":
			return <PasskeyAskCard personaId={receiver.id} event={event} teammate={receiver.name} />;
		default:
			return null;
	}
}
