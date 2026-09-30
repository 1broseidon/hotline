import type {
	BackendChoice,
	CatalogModel,
	ChapterSummary,
	Command,
	ComputerCapacity,
	ComputerReleases,
	ComputerStatus,
	ConfigChoice,
	CookieImport,
	CookieSite,
	Credential,
	EffortChoices,
	FileChunk,
	GlobalSearchHit,
	HostBrowser,
	LoginPrompt,
	LoginStatus,
	PasskeyRegistration,
	PeerThreadSummary,
	Persona,
	Provider,
	Report,
	RemoteDevice,
	RemotePairing,
	RemoteStatus,
	SealedPairing,
	RosterEntry,
	RuntimeReport,
	ScheduledJob,
	SessionInfo,
	SharedSecret,
	SkillEntry,
	StreamDelta,
	Target,
	TeammateToolLedger,
	ThreadSearchHit,
	TranscriptEvent,
	VoiceCall,
	VoiceStatus,
	Welcome,
} from "./generated/contract";
import { activeDeskId, allDesks, wireFor } from "./desks";

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

/** OAuth gateway status; access and refresh tokens never cross this type. */
export type McpOAuthStatus = {
	serverId: string;
	status: "signed_out" | "pending" | "signed_in" | "failed";
	loginId?: string;
	authorizationUrl?: string;
	redirectUri?: string;
	error?: string;
};

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
    "remote.status": RemoteStatus;
    "remote.configure": RemoteStatus;
    "remote.devices": RemoteDevice[];
    "remote.revoke": RemoteStatus;
    "remote.pairing": RemotePairing | SealedPairing | RemoteDevice | null;
    "agent.auth.start": { id: string };
    "agent.auth.poll": { state: "running" | "succeeded" | "failed"; output: string; error?: string };
    "agent.auth.input": null;
    "agent.auth.cancel": null;
	"persona.create": Persona;
	"persona.update": Persona;
	"persona.delete": null;
	"settings.update": Record<string, unknown>;
	"credential.create": Credential;
	"credential.login": LoginPrompt;
	"credential.login_cancel": null;
	"credential.connect_local": Credential;
	"credential.custom_save": Credential;
	"credential.custom_models": string[];
	"credential.login_status": LoginStatus;
	"credential.refresh_models": CatalogModel[];
	"credential.revoke": null;
	"credential.delete": null;
	"credential.list": Credential[];
	"mcp.auth_start": McpOAuthStatus;
	"mcp.auth_callback": McpOAuthStatus;
	"mcp.auth_status": McpOAuthStatus;
	"mcp.auth_reconnect": McpOAuthStatus;
	"mcp.auth_sign_out": null;
	"mcp.secret_set": null;
	"backends.list": BackendChoice[];
	"skills.list": SkillEntry[];
	"skills.add": SkillEntry;
	"skills.remove": null;
	"skills.offer": SkillEntry;
	"providers.list": Provider[];
	"models.list": ConfigChoice[];
	"models.catalog": CatalogModel[];
	"models.manual_set": CatalogModel[];
	"models.efforts": EffortChoices;
	"session.start": SessionInfo;
	"session.stop": null;
	"session.prompt": null;
	"mobile.prompt": { state: "accepted" | "unknown" };
	"mobile.attachment": { offset: number; complete: boolean };
	"mobile.push_register": null;
	"mobile.persona_create": Persona;
	"mobile.persona_update": Persona;
	"mobile.persona_access": Persona;
	"mobile.persona_computer": Persona;
	"session.cancel": null;
	"session.set_model": SessionInfo;
	"session.set_mode": SessionInfo;
	"session.set_config": SessionInfo;
	"session.answer_permission": null;
	"teammates.exchange_resume": null;
	"teammates.exchange_stop": null;
	"human.answer": null;
	"search.thread": ThreadSearchResult;
	/** Older lines of a tape than its window, oldest first. */
	"tape.page": { events: TranscriptEvent[]; more: boolean };
	"search.all": GlobalSearchResult;
	/** One part of a file a teammate sent, by its message. */
	"file.read": FileChunk;
	"files.browse": {
		path: string;
		parent: string | null;
		entries: { name: string; path: string; directory: boolean; size: number }[];
	};
	"files.mkdir": { path: string };
	"files.download": Omit<FileChunk, "next"> & { next: number | null };
	"files.upload_start": { uploadId: string; offset: number; path: string };
	"files.upload_chunk": { offset: number };
	"files.upload_finish": { path: string; size: number };
	"files.upload_cancel": null;
	"chapter.list": ChapterSummary[];
	"chapter.start_fresh": ChapterSummary;
	"chapter.resume": ChapterSummary;
	"room.import": Report;
	"teammate.tools": TeammateToolLedger | null;
	"schedule.create": ScheduledJob;
	"schedule.list": ScheduledJob[];
	"schedule.cancel": null;
	"schedule.set_quiet": null;
	"peers.list": PeerThreadSummary[];
	/** How many bubbles that receipt actually moved. */
	"peers.mark_read": number;
	"computer.capacity": ComputerCapacity;
	"computer.runtimes": RuntimeReport[];
	"computer.releases": ComputerReleases;
	"computer.releases.check": ComputerReleases;
	"computer.status": ComputerStatus;
	"computer.stop": null;
	"computer.remove": null;
	"computer.update": null;
	"computer.browsers.list": HostBrowser[];
	"computer.cookies.preview": CookieSite[];
	"computer.cookies.import": CookieSite[];
	"computer.cookies.push": CookieSite[];
	"computer.cookies.list": CookieImport[];
	"computer.cookies.forget": CookieImport[];
	/** Names, kinds and what each is for; never a value. */
	"secrets.list": SharedSecret[];
	"secrets.set": SharedSecret;
	"secrets.login.set": SharedSecret;
	"secrets.delete": null;
	/** Where the making of a teammate's passkey stands. */
	"secrets.passkey.register": PasskeyRegistration;
	"secrets.passkey.registration": PasskeyRegistration;
	"secrets.passkey.answer": PasskeyRegistration;
	"secrets.passkey.cancel": null;
	"desk.looking": null;
	welcome: Welcome;
	"voice.status": VoiceStatus;
	"voice.call_start": VoiceCall;
	"voice.utterance": null;
	"voice.interrupt": null;
	"voice.hold": null;
	"voice.call_end": null;
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
		__hotlineDesk?: {
			platform: string;
			origin: string;
			token: string;
			version?: string;
			dataDir?: string;
			computerImage?: string;
		};
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

/** Where one desk answers: the local Door, or the shell's bridge to a remote desk. */
export type Endpoint = { origin: string; token: string };

/** One desk's connection. The window holds one per desk; see desks.ts. */
export class Wire {
	private socket: WebSocket | null = null;
	private closed = false;
	private nextId = 1;
	private readonly pending = new Map<number, Pending>();
	private readonly live = new Map<number, Live>();
	private readonly watchers = new Set<(state: Connection) => void>();
	private failures = 0;
	private retry: ReturnType<typeof setTimeout> | null = null;
	private state: Connection = "closed";

	constructor(private readonly endpoint: Endpoint) {}

	/** Opens the socket, and keeps it open until `close`: for as long as the window lives, or the desk stays in it. */
	connect(): void {
		// A paired desk whose bridge has not started has no endpoint yet; the
		// shell's next list names one, and a fresh connection dials it.
		if (this.closed || this.socket || this.endpoint.origin === "") return;
		this.setState("connecting");
		const socket = new WebSocket(`${this.endpoint.origin}/ws?token=${encodeURIComponent(this.endpoint.token)}`);
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
				reject(new Error("Hotline is not connected."));
			}
		});
	}

	/**
	 * Watches a target until the returned function is called. The snapshot is
	 * delivered again after a reconnect, so a handler must be able to replace
	 * what it holds rather than add to it.
	 *
	 * send() is a no-op while the socket is still connecting. Callers that
	 * subscribe from a mount effect (a restored teammate, a thread pane)
	 * must wait for `open` — see watchWhenOpen in tape.ts — because a
	 * subscribe that lands in `live` and is then cleaned up before onopen
	 * is never asked for again.
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

	/** For good: the desk left the window, or its endpoint changed. Nothing reconnects. */
	close(): void {
		this.closed = true;
		if (this.retry !== null) clearTimeout(this.retry);
		this.retry = null;
		this.live.clear();
		const socket = this.socket;
		this.socket = null;
		socket?.close();
		this.setState("closed");
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
				console.error(`Hotline refused to watch ${JSON.stringify(sub.target)}: ${String(frame["error"])}`);
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
			waiting.reject(new Error("The connection to Hotline dropped."));
		}
		this.pending.clear();
		if (this.closed || this.retry !== null) return;
		const wait = BACKOFF_MS[Math.min(this.failures, BACKOFF_MS.length - 1)] ?? 8_000;
		this.failures++;
		this.retry = setTimeout(() => {
			this.retry = null;
			this.connect();
		}, wait);
	}
}

/**
 * The active desk's connection, for everything on screen: the conversation,
 * the teammate pane, settings. Each call reaches the desk that is active at
 * the moment of the call, and a subscription stays with the desk it was
 * made on (its unsubscribe is that desk's). Switching desks remounts what is
 * on screen (App keys it by desk), so nothing keeps talking to the old one.
 * Code that must reach a particular desk, whichever is active, uses
 * `wireFor(deskId)`.
 */
export const wire = {
	/** Connects to every desk the window holds. */
	connect(): void {
		for (const desk of allDesks()) wireFor(desk.id).connect();
	},
	onConnection(watcher: (state: Connection) => void): () => void {
		return activeWire()?.onConnection(watcher) ?? (watcher("closed"), () => {});
	},
	command<Name extends CommandName>(cmd: Name, params: Commands[Name]["params"]): Promise<Commands[Name]["result"]> {
		const one = activeWire();
		return one ? one.command(cmd, params) : Promise.reject(new Error("No desk is open in this window."));
	},
	subscribe<Item, Ephemeral = never>(target: Target, handlers: Handlers<Item, Ephemeral>): () => void {
		return activeWire()?.subscribe(target, handlers) ?? (() => {});
	},
};

function activeWire(): Wire | null {
	const id = activeDeskId();
	return id === null ? null : wireFor(id);
}
