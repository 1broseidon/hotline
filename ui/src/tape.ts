import { useEffect, useState } from "react";
import type { StreamDelta, TranscriptEvent } from "./generated/contract";
import { bubbleId, pacedLive } from "./pacing";
import { wire, type Connection, type Target } from "./wire";

/**
 * Text arriving as the agent writes it, before the message it belongs to has
 * been written down. Keyed by the id the real event will carry, which is how
 * the in-progress bubble knows when it has been superseded.
 */
export type Streaming = {
	messageId: string;
	kind: "agent" | "thought";
	text: string;
	/**
	 * The rest of a streamed reply whose first bubbles the desk has already
	 * written: it draws as bubbles `index`, `index + 1`, … of `base` until
	 * their durable twins land, so the screen never loses them in between.
	 */
	bubbleOf?: { base: string; index: number };
};

/**
 * One teammate's conversation, folded by event id.
 *
 * A stream is append-only and superseding: a tool call moving from pending to
 * completed is a second line with the same id. Folding is the reader's job, so
 * every reader agrees on what the tape says, and it is also what makes a
 * reconnect's second snapshot harmless.
 */
export function useTape(personaId: string): { events: TranscriptEvent[]; streaming: Streaming[]; loaded: boolean } {
	const [events, setEvents] = useState<TranscriptEvent[]>([]);
	const [streaming, setStreaming] = useState<Streaming[]>([]);
	/* Whether the snapshot has landed: an empty tape and a tape not yet read
	 * look the same, and the starter card must not show on the second. */
	const [loaded, setLoaded] = useState(false);

	useEffect(() => {
		setEvents([]);
		setStreaming([]);
		setLoaded(false);
		return watchWhenOpen<TranscriptEvent, StreamDelta>({ tape: personaId }, {
			snapshot: (items) => {
				setEvents(fold(items));
				setLoaded(true);
			},
			event: (item) => {
				setEvents((known) => merge(known, item));
				// The durable line has landed, so the bubble Hotline was drawing
				// for it is no longer the best thing it has; what it was
				// drawing after that bubble stays until its own line lands.
				setStreaming((live) => settle(live, item));
			},
			ephemeral: (delta) => setStreaming((live) => append(live, delta)),
		});
	}, [personaId]);

	return { events, streaming, loaded };
}

/**
 * A thread is a stream like a tape: one snapshot, then events folded by id.
 * No ephemeral deltas — a peer turn does not stream into this pane.
 */
export function useThread(key: string): { events: TranscriptEvent[] } {
	const [events, setEvents] = useState<TranscriptEvent[]>([]);

	useEffect(() => {
		setEvents([]);
		return watchWhenOpen<TranscriptEvent>({ thread: key }, {
			snapshot: (items) => setEvents(fold(items)),
			event: (item) => setEvents((known) => merge(known, item)),
		});
	}, [key]);

	return { events };
}

/**
 * Subscribe only once the socket is open.
 *
 * The race: a restored teammate mounts Conversation in the same turn as
 * wire.connect(), so useTape calls subscribe() while the socket is still
 * CONNECTING. send() returns false. React then remounts (Strict Mode, or
 * the roster snapshot replacing a stub), the cleanup drops the live
 * entry, and onopen replays an empty map. The column stays on "Nothing
 * said yet" until the row is clicked again. Waiting for `open` means
 * the subscribe is sent once, when it can land.
 */
function watchWhenOpen<Item, Ephemeral = never>(
	target: Target,
	handlers: {
		snapshot(items: Item[]): void;
		event(item: Item): void;
		ephemeral?(frame: Ephemeral): void;
	},
): () => void {
	let unsub: (() => void) | undefined;
	const stop = wire.onConnection((state: Connection) => {
		if (state !== "open" || unsub) return;
		unsub = wire.subscribe(target, handlers);
	});
	return () => {
		stop();
		unsub?.();
	};
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

/**
 * A written line replaces the streamed bubble it covers. The desk writes one
 * reply as bubbles `m`, `m-2`, `m-3`, each its own frame, so the streamed text
 * after the covered bubble is kept as a remainder under the ids the siblings
 * will carry; each sibling then replaces exactly its own piece.
 */
function settle(live: Streaming[], item: TranscriptEvent): Streaming[] {
	const at = live.findIndex((one) => one.messageId === item.id);
	if (at === -1) return live;
	const streamed = live[at]!;
	const next = live.slice();
	next.splice(at, 1);
	if (streamed.kind !== "agent" || item.kind !== "agent") return next;
	const base = streamed.bubbleOf?.base ?? streamed.messageId;
	const index = streamed.bubbleOf?.index ?? 0;
	const cut = pacedLive(streamed.text);
	for (let k = 1; k < cut.length; k++) {
		if (cut.slice(0, k).join("\n\n") !== item.text) continue;
		next.splice(at, 0, {
			messageId: bubbleId(base, index + k),
			kind: "agent",
			text: cut.slice(k).join("\n\n"),
			bubbleOf: { base, index: index + k },
		});
		break;
	}
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
