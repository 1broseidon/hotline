import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
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
/** A computer download under way: layers landed of layers counted, 0 of 0 until the runtime says. */
export type Pulling = { done: number; total: number };

type TapeState = { events: TranscriptEvent[]; streaming: Streaming[]; loaded: boolean; pulling: Pulling | null };

/** How long a tape nobody is showing stays subscribed, so coming back to it draws at once. */
const LINGER_MS = 60_000;
/** Tapes kept in memory once unsubscribed, newest first, so a return draws the last known state. */
const KEEP = 8;

/**
 * One teammate's tape, shared by every pane that shows it.
 *
 * The conversation and the work pane beside it read the same store, so the
 * desk sends the snapshot once and every delta is folded once. When the last
 * reader goes the subscription lingers for a minute, and after that the
 * folded events are kept (up to KEEP tapes), so switching back to a teammate
 * draws what it last showed while the new snapshot is on its way instead of
 * an empty column.
 *
 * Streaming deltas arrive a word or two at a time, often faster than the
 * screen draws. They are queued and folded once per animation frame, so a
 * reply costs one render per frame, not one per token.
 */
class TapeStore {
	state: TapeState = { events: [], streaming: [], loaded: false, pulling: null };
	private readonly listeners = new Set<() => void>();
	private readers = 0;
	private unsub: (() => void) | null = null;
	private linger: ReturnType<typeof setTimeout> | null = null;
	private queued: Exclude<StreamDelta, { type: "computer_pull" }>[] = [];
	private frame: number | null = null;

	constructor(private readonly personaId: string) {}

	subscribe = (listener: () => void) => {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	};

	snapshot = () => this.state;

	get idle(): boolean {
		return this.readers === 0 && this.unsub === null;
	}

	acquire() {
		this.readers++;
		if (this.linger !== null) {
			clearTimeout(this.linger);
			this.linger = null;
		}
		if (this.unsub === null) this.open();
	}

	release() {
		this.readers--;
		if (this.readers > 0 || this.linger !== null) return;
		this.linger = setTimeout(() => {
			this.linger = null;
			if (this.readers > 0) return;
			this.close();
			forget();
		}, LINGER_MS);
	}

	private set(next: Partial<TapeState>) {
		this.state = { ...this.state, ...next };
		for (const listener of this.listeners) listener();
	}

	private open() {
		this.unsub = watchWhenOpen<TranscriptEvent, StreamDelta>({ tape: this.personaId }, {
			snapshot: (items) => {
				this.drop();
				// A snapshot is a (re)connect: a download that ended while the
				// socket was down said so on a frame nobody heard. Still going,
				// its next report draws the ring again. What was streaming is
				// either in the snapshot now or will stream again.
				this.set({ events: fold(items), streaming: [], pulling: null, loaded: true });
			},
			event: (item) => {
				// Deltas queued before this line belong before it.
				this.flush();
				// The durable line has landed, so the bubble Hotline was drawing
				// for it is no longer the best thing it has; what it was
				// drawing after that bubble stays until its own line lands.
				this.set({ events: merge(this.state.events, item), streaming: settle(this.state.streaming, item) });
			},
			ephemeral: (delta) => {
				// A download is drawn on the computer's button, not in the talk.
				if (delta.type === "computer_pull") {
					this.set({ pulling: delta.status === "pulling" ? { done: delta.layersDone, total: delta.layersTotal } : null });
					return;
				}
				this.queued.push(delta);
				this.frame ??= requestAnimationFrame(this.flush);
			},
		});
	}

	private close() {
		this.unsub?.();
		this.unsub = null;
		this.drop();
		// Kept for the next visit, but what was mid-stream is no longer known.
		if (this.state.streaming.length > 0 || this.state.pulling !== null) this.set({ streaming: [], pulling: null });
	}

	private flush = () => {
		if (this.frame !== null) cancelAnimationFrame(this.frame);
		this.frame = null;
		if (this.queued.length === 0) return;
		let streaming = this.state.streaming;
		for (const delta of this.queued) streaming = append(streaming, delta);
		this.queued = [];
		this.set({ streaming });
	};

	private drop() {
		if (this.frame !== null) cancelAnimationFrame(this.frame);
		this.frame = null;
		this.queued = [];
	}
}

const stores = new Map<string, TapeStore>();

function storeFor(personaId: string): TapeStore {
	let store = stores.get(personaId);
	if (store === undefined) {
		store = new TapeStore(personaId);
	} else {
		stores.delete(personaId);
	}
	// Most recent last, so the first key is the one to let go of.
	stores.set(personaId, store);
	return store;
}

/** Lets go of the longest-unvisited tapes nobody is reading, beyond KEEP. */
function forget() {
	for (const [id, store] of stores) {
		if (stores.size <= KEEP) break;
		if (store.idle) stores.delete(id);
	}
}

export function useTape(personaId: string): TapeState {
	const store = useMemo(() => storeFor(personaId), [personaId]);
	useEffect(() => {
		store.acquire();
		return () => store.release();
	}, [store]);
	return useSyncExternalStore(store.subscribe, store.snapshot);
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
 * A subagent's run is a stream like a thread: one snapshot, then events
 * folded by id. It opens on the run's own line, rewritten as the run goes,
 * which is how the pane knows the run's title and whether it is still going.
 */
export function useRun(runId: string): { events: TranscriptEvent[] } {
	const [events, setEvents] = useState<TranscriptEvent[]>([]);

	useEffect(() => {
		setEvents([]);
		return watchWhenOpen<TranscriptEvent>({ run: runId }, {
			snapshot: (items) => setEvents(fold(items)),
			event: (item) => setEvents((known) => merge(known, item)),
		});
	}, [runId]);

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

function append(live: Streaming[], delta: Exclude<StreamDelta, { type: "computer_pull" }>): Streaming[] {
	const kind = delta.type === "agent_delta" ? "agent" : "thought";
	const at = live.findIndex((one) => one.messageId === delta.messageId);
	if (at === -1) return [...live, { messageId: delta.messageId, kind, text: delta.text }];
	const next = live.slice();
	next[at] = { ...live[at]!, text: live[at]!.text + delta.text };
	return next;
}
