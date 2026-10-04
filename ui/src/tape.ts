import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";
import type { Attachment, StreamDelta, ThreadAnswer, ThreadId, ThreadSummary, TranscriptEvent } from "./generated/contract";
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

/** A computer download under way: layers landed of layers counted, 0 of 0 until the runtime says. */
export type Pulling = { done: number; total: number };

/**
 * One thread's conversation, folded by event id.
 *
 * A stream is append-only and superseding: a tool call moving from pending to
 * completed is a second line with the same id. Folding is the reader's job, so
 * every reader agrees on what the thread says, and it is also what makes a
 * reconnect's second snapshot harmless.
 */
export type ThreadState = {
	events: TranscriptEvent[];
	streaming: Streaming[];
	loaded: boolean;
	/** A computer download, drawn on a teammate's button: only a DM hears of one. */
	pulling: Pulling | null;
	/** Older lines are on the desk that this window has not loaded yet. */
	more: boolean;
};

/** A thread's words arriving: the one live delta every kind is sent. */
export type ThreadDelta = Extract<StreamDelta, { type: "thread_delta" }>;

/** The state with what the store keeps beside it, so a reconnect's snapshot does not lose the pages loaded above it. */
type Held = ThreadState & {
	/** Lines loaded above the window by `earlier`, oldest first. */
	older: TranscriptEvent[];
	olderMore: boolean;
};

/** What can happen to a thread's lines on this window. */
export type Fold =
	| { type: "snapshot"; items: TranscriptEvent[] }
	| { type: "event"; item: TranscriptEvent }
	| { type: "words"; deltas: ThreadDelta[] }
	| { type: "page"; events: TranscriptEvent[]; more: boolean }
	| { type: "pull"; pulling: Pulling | null }
	| { type: "left" };

export const NOTHING_YET: Held = { events: [], streaming: [], loaded: false, pulling: null, more: false, older: [], olderMore: false };

/**
 * The one place a thread's state changes, whatever its kind: a snapshot (a
 * connect, or a reconnect's second one) replaces the window and keeps the
 * pages above it, an event lands by id, and words stream until the line
 * that carries them is written. Returns `held` itself when nothing changed.
 */
export function reduceThread(held: Held, fold: Fold): Held {
	switch (fold.type) {
		case "snapshot": {
			// A snapshot is a (re)connect: a download that ended while the
			// socket was down said so on a frame nobody heard. Still going,
			// its next report draws the ring again. What was streaming is
			// either in the snapshot now or will stream again.
			const window = foldById(fold.items);
			const inWindow = new Set(window.map((one) => one.id));
			const older = held.older.filter((one) => !inWindow.has(one.id));
			const more = older.length > 0 ? held.olderMore : window.length >= WINDOWED;
			return { ...held, events: [...older, ...window], streaming: [], pulling: null, loaded: true, more, older };
		}
		case "event":
			// The durable line has landed, so the bubble Hotline was drawing
			// for it is no longer the best thing it has; what it was
			// drawing after that bubble stays until its own line lands.
			return { ...held, events: mergeInWindow(held.events, fold.item, held.more), streaming: settle(held.streaming, fold.item) };
		case "words": {
			if (fold.deltas.length === 0) return held;
			let streaming = held.streaming;
			for (const delta of fold.deltas) streaming = append(streaming, delta);
			return { ...held, streaming };
		}
		case "page": {
			const known = new Set(held.events.map((one) => one.id));
			const page = foldById(fold.events).filter((one) => !known.has(one.id));
			return { ...held, events: [...page, ...held.events], older: [...page, ...held.older], olderMore: fold.more, more: fold.more };
		}
		case "pull":
			return { ...held, pulling: fold.pulling };
		case "left":
			// Kept for the next visit, but what was mid-stream is no longer known.
			return held.streaming.length > 0 || held.pulling !== null ? { ...held, streaming: [], pulling: null } : held;
	}
}

/**
 * A window opens on a thread's last lines (400 for the window, 200 for a
 * phone seat), not the whole of it: a busy teammate's tape runs to megabytes.
 * A snapshot this long may have more above it; a shorter one is the whole thing.
 */
const WINDOWED = 200;

/** How long a thread nobody is showing stays subscribed, so coming back to it draws at once. */
const LINGER_MS = 60_000;
/** Threads kept in memory once unsubscribed, newest first, so a return draws the last known state. */
const KEEP = 8;

/** Whether two names are the same thread. */
export const sameThread = (a: ThreadId, b: ThreadId): boolean => a.kind === b.kind && a.key === b.key;

/** A teammate's main conversation. */
export const dmOf = (personaId: string): ThreadId => ({ kind: "dm", key: personaId });

/**
 * One thread, shared by every pane that shows it.
 *
 * The conversation, the work card beside it and the dock read the same store,
 * so the desk sends the snapshot once and every delta is folded once. When the
 * last reader goes the subscription lingers for a minute, and after that the
 * folded events are kept (up to KEEP threads), so switching back draws what it
 * last showed while the new snapshot is on its way instead of an empty column.
 *
 * Streaming deltas arrive a word or two at a time, often faster than the
 * screen draws. They are queued and folded once per animation frame, so a
 * reply costs one render per frame, not one per token.
 */
class ThreadStore {
	private held: Held = NOTHING_YET;
	state: ThreadState = NOTHING_YET;
	private paging: Promise<void> | null = null;
	private readonly listeners = new Set<() => void>();
	private readers = 0;
	private unsub: (() => void) | null = null;
	private linger: ReturnType<typeof setTimeout> | null = null;
	private queued: ThreadDelta[] = [];
	private frame: number | null = null;

	constructor(
		private readonly wire: Wire,
		readonly id: ThreadId,
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

	private apply(fold: Fold) {
		const next = reduceThread(this.held, fold);
		if (next === this.held) return;
		this.held = next;
		const { events, streaming, loaded, pulling, more } = next;
		this.state = { events, streaming, loaded, pulling, more };
		for (const listener of this.listeners) listener();
	}

	private open() {
		this.unsub = watchWhenOpen<TranscriptEvent, StreamDelta>(this.wire, { threadId: this.id }, {
			snapshot: (items) => {
				this.drop();
				this.apply({ type: "snapshot", items });
			},
			event: (item) => {
				// Deltas queued before this line belong before it.
				this.flush();
				this.apply({ type: "event", item });
			},
			ephemeral: (delta) => {
				// A download is drawn on the computer's button, not in the talk.
				if (delta.type === "computer_pull") {
					this.apply({ type: "pull", pulling: delta.status === "pulling" ? { done: delta.layersDone, total: delta.layersTotal } : null });
					return;
				}
				// The window declared `threads2`: this is the only delta it is sent.
				if (delta.type !== "thread_delta" || !sameThread(delta.thread, this.id)) return;
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
			.command("thread.page", { thread: this.id, before: first.id, ...(through !== undefined ? { through } : {}) })
			.then(({ events, more }) => this.apply({ type: "page", events, more }))
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
		this.apply({ type: "left" });
	}

	private flush = () => {
		if (this.frame !== null) cancelAnimationFrame(this.frame);
		this.frame = null;
		if (this.queued.length === 0) return;
		const deltas = this.queued;
		this.queued = [];
		this.apply({ type: "words", deltas });
	};

	private drop() {
		if (this.frame !== null) cancelAnimationFrame(this.frame);
		this.frame = null;
		this.queued = [];
	}
}

const stores = new Map<string, ThreadStore>();

/** Two desks can each have a thread with the same id; a thread is one desk's. */
function storeFor(deskId: string, id: ThreadId): ThreadStore {
	const key = `${deskId}\u0000${id.kind}\u0000${id.key}`;
	let store = stores.get(key);
	if (store === undefined) {
		store = new ThreadStore(wireFor(deskId), id);
	} else {
		stores.delete(key);
	}
	// Most recent last, so the first key is the one to let go of.
	stores.set(key, store);
	return store;
}

/** Lets go of the longest-unvisited threads nobody is reading, beyond KEEP. */
function forget() {
	for (const [id, store] of stores) {
		if (stores.size <= KEEP) break;
		if (store.idle) stores.delete(id);
	}
}

const NOBODY = { subscribe: () => () => {}, snapshot: () => NOTHING_YET as ThreadState };

/** What a reader of a thread can do to it, bound to its name. */
export type ThreadHandle = ThreadState & {
	earlier(through?: string): Promise<void>;
	/** Says something in it: a work thread, or the main conversation. */
	prompt(text: string, attachments?: Attachment[]): Promise<null>;
	/** Stops the turn in flight; the thread stays as it was. */
	cancel(): Promise<null>;
	/** Ends a work thread, its transcript kept. */
	close(): Promise<null>;
	/** A parked or closed work thread, live again. */
	resume(): Promise<ThreadSummary>;
};

/**
 * Any thread by its name, whatever its kind: the main conversation, a work
 * thread, an exchange between two teammates, a subagent's run, a call. One
 * subscription (`{threadId}`), one fold, one set of verbs. `null` is no thread
 * yet, and reads as an empty one.
 */
export function useThread(id: ThreadId | null): ThreadHandle {
	// The active desk's: switching desks remounts the window, and the
	// store it comes back to is the other desk's.
	const deskId = activeDeskId() ?? "";
	const kind = id?.kind;
	const key = id?.key;
	const store = useMemo(() => (kind === undefined || key === undefined ? null : storeFor(deskId, { kind, key })), [deskId, kind, key]);
	useEffect(() => {
		if (store === null) return;
		store.acquire();
		return () => store.release();
	}, [store]);
	const state = useSyncExternalStore(store?.subscribe ?? NOBODY.subscribe, store?.snapshot ?? NOBODY.snapshot);
	const thread = useMemo(() => (kind === undefined || key === undefined ? null : { kind, key }), [kind, key]);
	const there = useCallback(
		<T,>(use: (wire: Wire, thread: ThreadId) => Promise<T>): Promise<T> =>
			thread === null ? Promise.reject(new Error("No thread is open.")) : use(wireFor(deskId), thread),
		[deskId, thread],
	);
	return useMemo(
		() => ({
			...state,
			earlier: store?.earlier ?? (() => Promise.resolve()),
			prompt: (text, attachments) =>
				there((wire, thread) => wire.command("thread.prompt", { thread, text, ...(attachments !== undefined && attachments.length > 0 ? { attachments } : {}) })),
			cancel: () => there((wire, thread) => wire.command("thread.cancel", { thread })),
			close: () => there((wire, thread) => wire.command("thread.close", { thread })),
			resume: () => there((wire, thread) => wire.command("thread.continue", { thread })),
		}),
		[state, store, there],
	);
}

/** A card in any thread is answered the same way: by naming the thread and the card. */
export function answerCard(thread: ThreadId, answer: ThreadAnswer): Promise<null> {
	return wireFor(activeDeskId() ?? "").command("thread.answer", { thread, answer });
}

/**
 * Subscribe only once the socket is open.
 *
 * The race: a restored teammate mounts Conversation in the same turn as
 * wire.connect(), so useThread calls subscribe() while the socket is still
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

function foldById(items: TranscriptEvent[]): TranscriptEvent[] {
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
export function settle(live: Streaming[], item: TranscriptEvent): Streaming[] {
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

/** Words arriving become a bubble of the kind they are, keyed by the id the real line will carry. */
export function append(live: Streaming[], delta: ThreadDelta): Streaming[] {
	const kind = delta.kind === "text" ? "agent" : "thought";
	const at = live.findIndex((one) => one.messageId === delta.messageId);
	if (at === -1) return [...live, { messageId: delta.messageId, kind, text: delta.text }];
	const next = live.slice();
	next[at] = { ...live[at]!, text: live[at]!.text + delta.text };
	return next;
}
