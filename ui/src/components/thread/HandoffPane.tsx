import { useMemo } from "react";
import type { ThreadId, TranscriptEvent } from "../../generated/contract";
import type { Person } from "../../avatars";
import type { Streaming } from "../../tape";
import { toBlocks, withStreaming } from "../../transcriptFolds";
import { messageWords } from "../../inbox";
import { TaskCard } from "./Pane";
import { WorkStream, type Worker } from "./WorkStream";
import { SectionRule, type Status } from "./Spine";
type Delivery = Extract<TranscriptEvent, { kind: "delivery" }>;

/** The delivery that handed the thread over: the brief its card shows. */
function handoffRequest(events: readonly TranscriptEvent[]): Delivery | undefined {
	return events.find((event): event is Delivery => event.kind === "delivery" && event.cause.kind === "handoff");
}

/**
 * The body of a thread a teammate handed over (docs/design/thread-panes.html,
 * Handoff), a 0.49 handoff or work taken from an inbox item alike: the brief
 * at the top, whole and from its sender, then under Activity, which carries
 * the page's status mark, the receiver's steps and words as they happen, the
 * way a subagent's pane shows its run, in the stream they share
 * (WorkStream). One press on the thread lands here: nothing in it is a row
 * that leads to the thread (George, 2026-11-21). A handoff's words go back
 * to the sender, so each says so ("Poe → Elvis"); the person's own lines are
 * theirs, and a card the receiver raises is answered here as anywhere. The
 * band and the composer stay the thread's.
 */
export function HandoffPane({
	thread,
	events,
	streaming,
	live,
	status,
	receiver,
	sender,
	people,
	brief,
	onOpenBrief,
}: {
	thread: ThreadId;
	/** The thread's lines, its link left out. */
	events: TranscriptEvent[];
	streaming: Streaming[];
	/** A turn of the receiver's is running. */
	live: boolean;
	/** How the thread's work stands, as the mark on Activity says it. */
	status: Status | undefined;
	receiver: Worker;
	sender: { personaId: string; name: string };
	people: ReadonlyMap<string, Person>;
	/**
	 * Work taken from an inbox item: the item's words that seeded it, kept
	 * out of `events`, under "Message" with the way to the item; its
	 * teammate's words are its own, not sent back. Absent for a 0.49
	 * handoff, whose request is read from `events`.
	 */
	brief?: Delivery | undefined;
	/** Opens the item the brief came from, by "Open ›" on its rule. */
	onOpenBrief?: (() => void) | undefined;
}) {
	const request = brief ?? handoffRequest(events);
	// Words an item seeded, whether or not the view still holds the item: the teammate's own work, said in its own name.
	const fromItem = brief !== undefined || (request !== undefined && messageWords(request.text) !== request.text);
	const blocks = useMemo(
		// Each run of steps where it fell between the words, as the receiver works.
		() => withStreaming(toBlocks(events.filter((event) => event !== request), true), streaming),
		[events, request, streaming],
	);
	return (
		<>
			{request !== undefined && (
				<TaskCard
					eyebrow={fromItem ? "Message" : "Request"}
					from={{ personaId: sender.personaId, name: sender.name, hash: people.get(sender.personaId)?.hash }}
					text={messageWords(request.text)}
					onOpen={onOpenBrief}
				/>
			)}
			<SectionRule title="Activity" status={status} />
			<WorkStream
				thread={thread}
				blocks={blocks}
				live={live}
				worker={receiver}
				said={(event) =>
					event.kind === "agent"
						? { name: fromItem ? receiver.name : `${receiver.name} → ${sender.name}`, mine: false }
						: event.kind === "user"
							? { name: "You", mine: true }
							: // A follow-up the sender sent into the thread, or a colleague's answer: in their name.
								event.cause.kind === "answer"
								? null
								: { name: `${event.cause.name} → ${receiver.name}`, mine: false }
				}
			/>
		</>
	);
}
