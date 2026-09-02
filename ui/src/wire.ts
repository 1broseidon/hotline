import type {
	BackendChoice,
	ChapterSummary,
	Command,
	ConfigChoice,
	Credential,
	GlobalSearchHit,
	PeerThreadSummary,
	Persona,
	Report,
	RosterEntry,
	ScheduledJob,
	SessionInfo,
	StreamDelta,
	Target,
	TeammateToolLedger,
	ThreadSearchHit,
	TranscriptEvent,
} from "./generated/contract";

export type { RosterEntry, Target };

/**
 * The window's one way of speaking to the core.
 *
 * Command names and params come from the generated `Command` union, so a
 * name the core does not know, or a field it renamed, is a type error here
 * and at every caller. Results are not generated yet, so they stay in the
 * table below — the only hand-written piece of the command surface.
 */

// ---------------------------------------------------------------------------
// The shapes the core has not exported yet
// ---------------------------------------------------------------------------

/** `search.thread`'s answer: what matched, and whether the index stopped early. */
export type ThreadSearchResult = { hits: ThreadSearchHit[]; truncated: boolean };

/** `search.all`'s answer: the same hits, each named with whose tape they came from. */
export type GlobalSearchResult = { hits: GlobalSearchHit[]; truncated: boolean };

// ---------------------------------------------------------------------------
// The command surface
// ---------------------------------------------------------------------------

export type CommandName = Command["cmd"];

type Params<N extends CommandName> = Extract<Command, { cmd: N }> extends {
	params: infer P;
}
	? P
	: Record<string, never>;

/**
 * What each command answers. The core does not generate result types yet, so
 * this table is the window's one remaining spelling of the reply.
 */
type Results = {
	"persona.create": Persona;
	"persona.update": Persona;
	"persona.delete": null;
	"settings.update": Record<string, unknown>;
	"credential.create": Credential;
	"credential.revoke": null;
	"credential.delete": null;
	"credential.list": Credential[];
	"backends.list": BackendChoice[];
	"models.list": ConfigChoice[];
	"session.start": SessionInfo;
	"session.stop": null;
	"session.prompt": null;
	"session.cancel": null;
	"session.set_model": SessionInfo;
	"session.set_mode": SessionInfo;
	"session.answer_permission": null;
	"search.thread": ThreadSearchResult;
	"search.all": GlobalSearchResult;
	"chapter.list": ChapterSummary[];
	"chapter.start_fresh": ChapterSummary;
	"room.import": Report;
	"teammate.tools": TeammateToolLedger | null;
	"schedule.create": ScheduledJob;
	"schedule.list": ScheduledJob[];
	"schedule.cancel": null;
	"schedule.set_quiet": null;
	"peers.list": PeerThreadSummary[];
	/** How many bubbles that receipt actually moved. */
	"peers.mark_read": number;
};

/** Every command the window may send, with what it sends and what it gets back. */
export type Commands = {
	[N in CommandName]: { params: Params<N>; result: Results[N] };
};

/**
 * What arrives on a subscription. `snapshot` lands exactly once, before any
 * event; `event` carries one item, which supersedes an earlier item with the
 * same id, because every stream is folded by id. `ephemeral` is a frame that
 * was never written down — a streaming delta — and `removed` is a tombstone.
 */
export type Handlers<Item, Ephemeral = never> = {
	snapshot(items: Item[]): void;
	event(item: Item): void;
	ephemeral?(frame: Ephemeral): void;
	removed?(id: string): void;
};

/** What the roster subscription carries. */
export type RosterHandlers = Handlers<RosterEntry>;
/** What a tape subscription carries. */
export type TapeHandlers = Handlers<TranscriptEvent, StreamDelta>;

// ---------------------------------------------------------------------------
// The door the shell opened for us
// ---------------------------------------------------------------------------

declare global {
	interface Window {
		__toadDesk?: { platform: string; origin: string; token: string };
	}
}

export type Connection = "connecting" | "open" | "closed";

type Pending = { resolve(value: unknown): void; reject(error: Error): void };

type Live = {
	target: Target;
	handlers: Handlers<unknown, unknown>;
};

/** How long to wait before dialling again, growing with each failure. */
const BACKOFF_MS = [250, 500, 1_000, 2_000, 4_000, 8_000];

class Wire {
	private socket: WebSocket | null = null;
	private nextId = 1;
	private readonly pending = new Map<number, Pending>();
	private readonly live = new Map<number, Live>();
	private readonly watchers = new Set<(state: Connection) => void>();
	private failures = 0;
	private retry: ReturnType<typeof setTimeout> | null = null;
	private state: Connection = "closed";

	/** Opens the socket, and keeps it open for as long as the window lives. */
	connect(): void {
		const desk = window.__toadDesk;
		if (!desk || this.socket) return;
		this.setState("connecting");
		const socket = new WebSocket(`${desk.origin}/ws?token=${encodeURIComponent(desk.token)}`);
		this.socket = socket;

		socket.onopen = () => {
			this.failures = 0;
			this.setState("open");
			// A subscription belongs to the window, not to the socket that
			// happened to carry it: everything still on screen is asked for
			// again, and each one answers with a fresh snapshot.
			for (const [id, sub] of this.live) this.send({ id, sub: sub.target });
		};
		socket.onmessage = (message) => this.receive(message.data);
		socket.onclose = () => this.drop();
		socket.onerror = () => socket.close();
	}

	onConnection(watcher: (state: Connection) => void): () => void {
		this.watchers.add(watcher);
		watcher(this.state);
		return () => this.watchers.delete(watcher);
	}

	command<Name extends CommandName>(
		cmd: Name,
		params: Commands[Name]["params"],
	): Promise<Commands[Name]["result"]> {
		return new Promise((resolve, reject) => {
			const id = this.nextId++;
			this.pending.set(id, { resolve: resolve as (value: unknown) => void, reject });
			if (!this.send({ id, cmd, params })) {
				this.pending.delete(id);
				reject(new Error("Toad is not connected."));
			}
		});
	}

	/**
	 * Watches a target until the returned function is called. The snapshot is
	 * delivered again after a reconnect, so a handler must be able to replace
	 * what it holds rather than add to it.
	 */
	subscribe<Item, Ephemeral = never>(
		target: Target,
		handlers: Handlers<Item, Ephemeral>,
	): () => void {
		const id = this.nextId++;
		this.live.set(id, { target, handlers: handlers as Handlers<unknown, unknown> });
		this.send({ id, sub: target });
		return () => {
			if (!this.live.delete(id)) return;
			this.send({ id: this.nextId++, unsub: id });
		};
	}

	// ---------------------------------------------------------------- private

	private setState(next: Connection): void {
		if (this.state === next) return;
		this.state = next;
		for (const watcher of this.watchers) watcher(next);
	}

	private send(frame: unknown): boolean {
		if (this.socket?.readyState !== WebSocket.OPEN) return false;
		this.socket.send(JSON.stringify(frame));
		return true;
	}

	private receive(data: unknown): void {
		if (typeof data !== "string") return;
		const frame = JSON.parse(data) as Record<string, unknown>;

		if (typeof frame["sub"] === "number") {
			const sub = this.live.get(frame["sub"]);
			if (!sub) return;
			if ("snapshot" in frame) sub.handlers.snapshot(frame["snapshot"] as unknown[]);
			else if ("event" in frame) sub.handlers.event(frame["event"]);
			else if ("ephemeral" in frame) sub.handlers.ephemeral?.(frame["ephemeral"]);
			else if ("removed" in frame) sub.handlers.removed?.(frame["removed"] as string);
			return;
		}

		if (typeof frame["id"] !== "number") return;
		const waiting = this.pending.get(frame["id"]);
		if (!waiting) {
			// A subscription's ack shares the command reply's shape and its id.
			// Nothing awaits the yes; the no would otherwise leave an empty
			// screen with no account of why.
			const sub = this.live.get(frame["id"]);
			if (sub && frame["ok"] === false) {
				console.error(`Toad refused to watch ${JSON.stringify(sub.target)}: ${String(frame["error"])}`);
			}
			return;
		}
		this.pending.delete(frame["id"]);
		if (frame["ok"] === true) waiting.resolve(frame["result"] ?? null);
		else waiting.reject(new Error(String(frame["error"] ?? "The core refused that.")));
	}

	/**
	 * The socket went away. Everything waiting on an answer is told so at
	 * once — a promise that never settles is a button that stays greyed out
	 * forever — and the dial starts again, more slowly each time.
	 */
	private drop(): void {
		this.socket = null;
		this.setState("closed");
		for (const waiting of this.pending.values()) {
			waiting.reject(new Error("The connection to Toad dropped."));
		}
		this.pending.clear();
		if (this.retry !== null) return;
		const wait = BACKOFF_MS[Math.min(this.failures, BACKOFF_MS.length - 1)] ?? 8_000;
		this.failures++;
		this.retry = setTimeout(() => {
			this.retry = null;
			this.connect();
		}, wait);
	}
}

export const wire = new Wire();
