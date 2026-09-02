import { useEffect, useState } from "react";
import type { StreamDelta, TranscriptEvent } from "./generated/contract";
import { wire } from "./wire";

/**
 * Text arriving as the agent writes it, before the message it belongs to has
 * been written down. Keyed by the id the real event will carry, which is how
 * the in-progress bubble knows when it has been superseded.
 */
export type Streaming = { messageId: string; kind: "agent" | "thought"; text: string };

/**
 * One teammate's conversation, folded by event id.
 *
 * A stream is append-only and superseding: a tool call moving from pending to
 * completed is a second line with the same id. Folding is the reader's job, so
 * every reader agrees on what the tape says, and it is also what makes a
 * reconnect's second snapshot harmless.
 */
export function useTape(personaId: string): { events: TranscriptEvent[]; streaming: Streaming[] } {
	const [events, setEvents] = useState<TranscriptEvent[]>([]);
	const [streaming, setStreaming] = useState<Streaming[]>([]);

	useEffect(() => {
		setEvents([]);
		setStreaming([]);
		return wire.subscribe<TranscriptEvent, StreamDelta>(
			{ tape: personaId },
			{
				snapshot: (items) => setEvents(fold(items)),
				event: (item) => {
					setEvents((known) => merge(known, item));
					// The durable line has landed, so the bubble Toad was drawing
					// for it is no longer the best thing it has.
					setStreaming((live) => live.filter((one) => one.messageId !== item.id));
				},
				ephemeral: (delta) => setStreaming((live) => append(live, delta)),
			},
		);
	}, [personaId]);

	return { events, streaming };
}

function fold(items: TranscriptEvent[]): TranscriptEvent[] {
	const byId = new Map<string, TranscriptEvent>();
	for (const item of items) byId.set(item.id, item);
	return [...byId.values()];
}

function merge(known: TranscriptEvent[], item: TranscriptEvent): TranscriptEvent[] {
	const at = known.findIndex((one) => one.id === item.id);
	if (at === -1) return [...known, item];
	const next = known.slice();
	next[at] = item;
	return next;
}

function append(live: Streaming[], delta: StreamDelta): Streaming[] {
	const kind = delta.type === "agent_delta" ? "agent" : "thought";
	const at = live.findIndex((one) => one.messageId === delta.messageId);
	if (at === -1) return [...live, { messageId: delta.messageId, kind, text: delta.text }];
	const next = live.slice();
	next[at] = { ...live[at]!, text: live[at]!.text + delta.text };
	return next;
}
