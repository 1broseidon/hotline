import { useMemo } from "react";
import type { ThreadId, TranscriptEvent } from "../../generated/contract";
import type { Person } from "../../avatars";
import type { Streaming } from "../../tape";
import { toBlocks, withStreaming } from "../../transcriptFolds";
import { TaskCard } from "./Pane";
import { WorkStream, type Worker } from "./WorkStream";
type Delivery = Extract<TranscriptEvent, { kind: "delivery" }>;

/** The delivery that handed the thread over: the request its card shows. */
function handoffRequest(events: readonly TranscriptEvent[]): Delivery | undefined {
	return events.find((event): event is Delivery => event.kind === "delivery" && event.cause.kind === "handoff");
}

/**
 * A handoff's body (docs/design/thread-panes.html, Handoff): the request it
 * was handed, then the receiver's steps and words as they happen, the way a
 * subagent's pane shows its run, in the stream they share (WorkStream). The receiver's words go back to the sender,
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
	receiver: Worker;
	sender: { personaId: string; name: string };
	people: ReadonlyMap<string, Person>;
}) {
	const request = handoffRequest(events);
	const blocks = useMemo(
		// Each run of steps where it fell between the words, as the receiver works.
		() => withStreaming(toBlocks(events.filter((event) => event !== request), true), streaming),
		[events, request, streaming],
	);
	return (
		<>
			{request !== undefined && (
				<TaskCard eyebrow="Request" from={{ personaId: sender.personaId, name: sender.name, hash: people.get(sender.personaId)?.hash }} text={request.text} />
			)}
			<WorkStream
				thread={thread}
				blocks={blocks}
				live={live}
				worker={receiver}
				said={(event) =>
					event.kind === "agent"
						? { name: `${receiver.name} → ${sender.name}`, mine: false }
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
