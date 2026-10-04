import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import type { StreamDelta, TranscriptEvent } from "./generated/contract";
import { bubbleId, pacedLive } from "./pacing";
import { activeDeskId, wireFor } from "./desks";
import type { Connection, Target, Wire } from "./wire";

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

type TapeState = {
	events: TranscriptEvent[];
	streaming: Streaming[];
	loaded: boolean;
	pulling: Pulling | null;
	/** Older lines are on the desk that this window has not loaded yet. */
	more: boolean;
};

/**
 * The desk opens a tape on its last lines (400 for the window, 200 for a
 * phone seat), not the whole of it: a busy teammate's tape runs to megabytes.
 * A snapshot this long may have more above it; a shorter one is the whole tape.
 */
const WINDOWED = 200;

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
	state: TapeState = { events: [], streaming: [], loaded: false, pulling: null, more: false };
	/** Lines loaded above the window by `earlier`, oldest first, kept across a reconnect's snapshot. */
	private older: TranscriptEvent[] = [];
	private olderMore = false;
	private paging: Promise<void> | null = null;
	private readonly listeners = new Set<() => void>();
	private readers = 0;
	private unsub: (() => void) | null = null;
	private linger: ReturnType<typeof setTimeout> | null = null;
	private queued: TextDelta[] = [];
	private frame: number | null = null;

	constructor(
		private readonly wire: Wire,
		private readonly personaId: string,
	) {}

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
		this.unsub = watchWhenOpen<TranscriptEvent, StreamDelta>(this.wire, { tape: this.personaId }, {
			snapshot: (items) => {
				this.drop();
				// A snapshot is a (re)connect: a download that ended while the
				// socket was down said so on a frame nobody heard. Still going,
				// its next report draws the ring again. What was streaming is
				// either in the snapshot now or will stream again.
				const window = fold(items);
				const inWindow = new Set(window.map((one) => one.id));
				const older = this.older.filter((one) => !inWindow.has(one.id));
				const more = older.length > 0 ? this.olderMore : window.length >= WINDOWED;
				this.set({ events: [...older, ...window], streaming: [], pulling: null, loaded: true, more });
			},
			event: (item) => {
				// Deltas queued before this line belong before it.
				this.flush();
				// The durable line has landed, so the bubble Hotline was drawing
				// for it is no longer the best thing it has; what it was
				// drawing after that bubble stays until its own line lands.
				this.set({ events: mergeInWindow(this.state.events, item, this.state.more), streaming: settle(this.state.streaming, item) });
			},
			ephemeral: (delta) => {
				// A download is drawn on the computer's button, not in the talk.
				if (delta.type === "computer_pull") {
					this.set({ pulling: delta.status === "pulling" ? { done: delta.layersDone, total: delta.layersTotal } : null });
					return;
				}
				// A side thread's words are its own: never drawn into the main talk.
				if (delta.type === "side_agent_delta" || delta.type === "side_thought_delta") return;
				// The window has not declared `threads2` yet, so it is sent none.
				if (delta.type === "thread_delta") return;
				this.queued.push(delta);
				this.frame ??= requestAnimationFrame(this.flush);
			},
		});
	}

	/**
	 * Loads the lines above the window: one page, or, with `through`, back as
	 * far as that line (a search hit, a quoted reply). One request at a time.
	 */
	earlier = (through?: string): Promise<void> => {
		if (this.paging !== null) return this.paging;
		const first = this.state.events[0];
		if (!this.state.more || first === undefined) return Promise.resolve();
		this.paging = this.wire
			.command("tape.page", { personaId: this.personaId, before: first.id, ...(through !== undefined ? { through } : {}) })
			.then(({ events, more }) => {
				const known = new Set(this.state.events.map((one) => one.id));
				const page = fold(events).filter((one) => !known.has(one.id));
				this.older = [...page, ...this.older];
				this.olderMore = more;
				this.set({ events: [...page, ...this.state.events], more });
			})
			.catch(() => {
				// Nothing is lost: the window keeps what it has and can ask again.
			})
			.finally(() => {
				this.paging = null;
			});
		return this.paging;
	};

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

/** Two desks can each have a teammate with the same id; a tape is one desk's. */
function storeFor(deskId: string, personaId: string): TapeStore {
	const key = `${deskId}\u0000${personaId}`;
	let store = stores.get(key);
	if (store === undefined) {
		store = new TapeStore(wireFor(deskId), personaId);
	} else {
		stores.delete(key);
	}
	// Most recent last, so the first key is the one to let go of.
	stores.set(key, store);
	return store;
}

/** Lets go of the longest-unvisited tapes nobody is reading, beyond KEEP. */
function forget() {
	for (const [id, store] of stores) {
		if (stores.size <= KEEP) break;
		if (store.idle) stores.delete(id);
	}
}

export function useTape(personaId: string): TapeState & { earlier(through?: string): Promise<void> } {
	// The active desk's: switching desks remounts the conversation, and the
	// store it comes back to is the other desk's.
	const deskId = activeDeskId() ?? "";
	const store = useMemo(() => storeFor(deskId, personaId), [deskId, personaId]);
	useEffect(() => {
		store.acquire();
		return () => store.release();
	}, [store]);
	const state = useSyncExternalStore(store.subscribe, store.snapshot);
	return useMemo(() => ({ ...state, earlier: store.earlier }), [state, store]);
}

/**
 * A thread is a stream like a tape: one snapshot, then events folded by id.
 * No ephemeral deltas — a peer turn does not stream into this pane.
 */
export function useThread(key: string): { events: TranscriptEvent[] } {
	const [events, setEvents] = useState<TranscriptEvent[]>([]);

	useEffect(() => {
		setEvents([]);
		return watchWhenOpen<TranscriptEvent>(activeWire(), { thread: key }, {
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
		return watchWhenOpen<TranscriptEvent>(activeWire(), { run: runId }, {
			snapshot: (items) => setEvents(fold(items)),
			event: (item) => setEvents((known) => merge(known, item)),
		});
	}, [runId]);

	return { events };
}

/**
 * A side thread: a stream like a run, folded by id, with the words arriving
 * as the teammate writes them. It opens on its own marker line, rewritten as
 * the thread goes, which is how the card knows whether it is still live.
 * Deltas are for this side id alone; the main tape never hears them.
 */
export function useSide(sideId: string): { events: TranscriptEvent[]; streaming: Streaming[]; loaded: boolean } {
	const [state, setState] = useState<{ events: TranscriptEvent[]; streaming: Streaming[]; loaded: boolean }>({
		events: [],
		streaming: [],
		loaded: false,
	});

	useEffect(() => {
		setState({ events: [], streaming: [], loaded: false });
		return watchWhenOpen<TranscriptEvent, StreamDelta>(activeWire(), { side: sideId }, {
			snapshot: (items) => setState({ events: fold(items), streaming: [], loaded: true }),
			event: (item) =>
				setState((known) => ({
					events: merge(known.events, item),
					streaming: settle(known.streaming, item),
					loaded: true,
				})),
			ephemeral: (delta) => {
				if (delta.type !== "side_agent_delta" && delta.type !== "side_thought_delta") return;
				if (delta.sideId !== sideId) return;
				setState((known) => ({ ...known, streaming: append(known.streaming, delta) }));
			},
		});
	}, [sideId]);

	return state;
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
	wire: Wire,
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

/**
 * `merge`, for a window that may not hold the whole tape: a rewrite of a line
 * above the window (a chapter closing, an old card answered) is not new, and
 * does not belong at the bottom. It is left for when that line is loaded.
 */
function mergeInWindow(known: TranscriptEvent[], item: TranscriptEvent, more: boolean): TranscriptEvent[] {
	const first = known[0];
	if (more && first !== undefined && item.ts < first.ts && !known.some((one) => one.id === item.id)) return known;
	return merge(known, item);
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

/** Words arriving, whichever stream they are for: a tape's or a side thread's. */
type TextDelta = Extract<StreamDelta, { type: "agent_delta" | "thought_delta" | "side_agent_delta" | "side_thought_delta" }>;

function append(live: Streaming[], delta: TextDelta): Streaming[] {
	const kind = delta.type === "agent_delta" || delta.type === "side_agent_delta" ? "agent" : "thought";
	const at = live.findIndex((one) => one.messageId === delta.messageId);
	if (at === -1) return [...live, { messageId: delta.messageId, kind, text: delta.text }];
	const next = live.slice();
	next[at] = { ...live[at]!, text: live[at]!.text + delta.text };
	return next;
}

/** The desk on screen when a thread or run pane mounts; it remounts with the desk. */
function activeWire(): Wire {
	return wireFor(activeDeskId() ?? "");
}
