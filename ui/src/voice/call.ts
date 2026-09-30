import { useEffect, useState, useSyncExternalStore } from "react";
import { activeDeskId, allDesks, watchDesks, wireFor } from "../desks";
import { type Target, wire } from "../wire";
import type { VoiceEndReason, VoiceEvent } from "../generated/contract";
import { type CallAudio, webAudio } from "./audio";
import { TurnDetector } from "./turn";
import { WAV_RATE, downsample, encodeWav, rms, toBase64 } from "./wav";

/**
 * A call with the desk (BRO-146, the wire contract in BRO-168). You talk to
 * the desk, not to a teammate: the window hears a turn, sends it as one WAV,
 * and plays back the dispatcher's sentences as they come. It lives outside
 * React so a call carries on while you move between conversations; the
 * pane only draws it.
 */

export type CallPhase =
	| "connecting"
	| "listening"
	| "hearing"
	| "thinking"
	| "speaking"
	| "held"
	| "ended";

export type CallLine =
	| { kind: "you"; id: string; text: string }
	| { kind: "desk"; id: string; text: string; from?: string };

export type CallCard = { personaId: string; requestId: string; kind: string };

export type EndReason = VoiceEndReason;

export type CallSnapshot = {
	phase: CallPhase;
	lines: CallLine[];
	cards: CallCard[];
	/** Talk time: `base` ms banked, plus the time since `since` while the line is live and not on hold. */
	clock: { base: number; since: number | null };
	/** Why it ended, once it has. */
	ended?: EndReason;
	/** One sentence for a person, when something went wrong. */
	trouble?: string | undefined;
};

/** What the desk sends on `{call: id}`: the generated `VoiceEvent`. */
export type CallEvent = VoiceEvent;

export type Reach = "open" | "closed" | "gone";

/** The two things a call needs from a desk; the wire in the window, a fake in tests. */
export type CallTransport = {
	command(cmd: string, params: Record<string, unknown>): Promise<unknown>;
	subscribe(target: unknown, handlers: { snapshot(items: unknown[]): void; event(item: unknown): void }): () => void;
	/**
	 * Whether the desk can be reached: `closed` is out of reach for now,
	 * `gone` is removed or no longer pairs with this computer. A call
	 * without it cannot tell.
	 */
	onConnection?(watcher: (state: Reach) => void): () => void;
};

const wireTransport: CallTransport = {
	command: (cmd, params) => (wire.command as (c: string, p: unknown) => Promise<unknown>)(cmd, params),
	subscribe: (target, handlers) => wire.subscribe(target as Target, handlers),
};

/**
 * A call belongs to the desk it was placed on: moving to another desk in
 * the window leaves it talking to the first, and hanging up reaches it.
 */
function deskTransport(): CallTransport {
	const deskId = activeDeskId();
	if (deskId === null) return wireTransport;
	const one = wireFor(deskId);
	return {
		command: (cmd, params) => (one.command as (c: string, p: unknown) => Promise<unknown>)(cmd, params),
		subscribe: (target, handlers) => one.subscribe(target as Target, handlers),
		// A server desk is reached through the shell's bridge, whose socket
		// stays open while the server is down: its own state says more.
		onConnection: (watcher) => {
			let socket = "connecting";
			const report = () => {
				const desk = allDesks().find((candidate) => candidate.id === deskId);
				if (desk === undefined || desk.state === "revoked") return watcher("gone");
				const deskUp = desk.kind !== "remote" || desk.state === undefined || desk.state === "open";
				watcher(socket === "open" && deskUp ? "open" : "closed");
			};
			const unwire = one.onConnection((state) => {
				socket = state;
				report();
			});
			const undesk = watchDesks(report);
			return () => {
				unwire();
				undesk();
			};
		},
	};
}

const ENDED_WORDS: Record<EndReason, string | undefined> = {
	client: undefined,
	goodbye: undefined,
	idle: "The call ended after ten quiet minutes.",
	budget: "Today's voice budget is spent. Carry on by text.",
	replaced: "The call moved to another device.",
	error: "The desk dropped the call.",
};

const LOST = "Lost the connection to the desk.";

/** How much audio before the detector is sure it heard speech is kept, so a word's first sound is not clipped. */
const PREROLL_MS = 400;

type Names = (personaId: string) => string | undefined;

export class Call {
	readonly id = crypto.randomUUID();
	private snapshot: CallSnapshot = { phase: "connecting", lines: [], cards: [], clock: { base: 0, since: null } };
	private readonly listeners = new Set<() => void>();
	private readonly levels = new Set<(level: number) => void>();
	private readonly audio: CallAudio;

	private detector: TurnDetector;
	private frames: Float32Array[] = [];
	private rate = 48_000;
	private heard = false;
	private seq = 0;
	private unsubscribe: (() => void) | null = null;
	private unwatch: (() => void) | null = null;
	/** Whether the desk has made the call; before it, there is nothing to hang up there. */
	private started = false;
	/** The desk's own word on where the call stands. */
	private desk: "listening" | "thinking" | "speaking" | "held" = "listening";
	/** An utterance is on its way and the desk has not answered with a state yet. */
	private awaiting = false;
	private held = false;
	/** Sentences from a turn that was cut in on: their late clips are not played. */
	private readonly muted = new Set<string>();
	private readonly seen = new Set<string>();
	private pendingFrom: string | undefined;
	/** The desk has ended the call; this waits for the last clip to finish first. */
	private closing: EndReason | null = null;
	private level = 0;
	private shown = 0;
	private raf = 0;

	constructor(
		private readonly transport: CallTransport = wireTransport,
		private readonly names: Names = () => undefined,
		audio?: CallAudio,
		private readonly now: () => number = () => performance.now(),
	) {
		this.audio = audio ?? webAudio({ onIdle: () => this.settle(), onLost: () => void this.micLost() });
		this.detector = new TurnDetector(this.now());
	}

	// ------------------------------------------------------------- reading

	get current(): CallSnapshot {
		return this.snapshot;
	}

	watch(listener: () => void): () => void {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	}

	/** The loudness to draw, 0..1, about sixty times a second: yours while you talk, the desk's while it does. */
	watchLevel(listener: (level: number) => void): () => void {
		this.levels.add(listener);
		return () => this.levels.delete(listener);
	}

	// ------------------------------------------------------------- acting

	/** From a press: the webview only lets audio start on a gesture. */
	async start(): Promise<void> {
		try {
			await this.audio.open((block, rate) => this.hear(block, rate));
		} catch {
			this.fail("Hotline can't hear the microphone. Allow it in your system settings, then call again.");
			return;
		}
		if (this.ended) return;
		// The call exists once the desk has answered call_start; only then can it be watched.
		try {
			await this.transport.command("voice.call_start", { callId: this.id });
		} catch (error) {
			this.fail(error instanceof Error ? error.message : String(error));
			return;
		}
		this.started = true;
		if (this.ended) {
			// Hung up while the desk was answering: tell it, now that it knows the call.
			void this.transport.command("voice.call_end", { callId: this.id }).catch(() => {});
			return;
		}
		this.unsubscribe = this.transport.subscribe(
			{ call: this.id },
			{
				// A snapshot is where the call stands, never audio to play again.
				snapshot: (items) => {
					for (const item of items) if ((item as CallEvent).type === "state") this.receive(item as CallEvent);
				},
				event: (item) => this.receive(item as CallEvent),
			},
		);
		this.unwatch = this.transport.onConnection?.((state) => this.connection(state)) ?? null;
		this.set({ clock: { base: 0, since: Date.now() } });
		this.audio.chime("connect");
		this.settle();
		this.tick();
	}

	hangUp(): void {
		if (this.ended) return;
		if (this.started) void this.transport.command("voice.call_end", { callId: this.id }).catch(() => {});
		this.end("client");
	}

	async hold(on: boolean): Promise<void> {
		if (this.ended || this.snapshot.phase === "connecting" || on === this.held) return;
		if (this.closing !== null) return this.end(this.closing);
		void this.transport.command("voice.hold", { callId: this.id, hold: on }).catch(() => {});
		if (on) {
			this.held = true;
			this.audio.stopPlayback();
			// Let the microphone go, so the system's mic light says so too.
			this.audio.closeMic();
			this.pauseClock();
			this.settle();
			return;
		}
		try {
			await this.audio.reopenMic();
		} catch {
			this.fail("Hotline can't hear the microphone any more. Call again when it's back.");
			return;
		}
		this.held = false;
		this.resumeClock();
		this.settle();
	}

	/** Cuts the desk off mid-sentence. It never stops a teammate's turn. */
	interrupt(): void {
		const phase = this.snapshot.phase;
		if (phase !== "speaking" && phase !== "thinking") return;
		if (this.closing !== null) return this.end(this.closing);
		void this.transport.command("voice.interrupt", { callId: this.id }).catch(() => {});
		for (const id of this.seen) this.muted.add(id);
		this.audio.stopPlayback();
		this.awaiting = false;
		// The desk says where it stands next; until then, listen.
		this.desk = "listening";
		this.settle();
	}

	dismissCard(requestId: string): void {
		this.set({ cards: this.snapshot.cards.filter((card) => card.requestId !== requestId) });
	}

	// ------------------------------------------------------------- the desk

	/** Exposed for tests; the subscription is the only caller in the window. */
	receive(event: CallEvent): void {
		if (this.ended) return;
		switch (event.type) {
			case "state":
				this.awaiting = false;
				if (event.state === "ended") {
					const reason = event.reason ?? "error";
					// A goodbye or a spent budget is said before the line goes: let the last sentence finish.
					if ((reason === "goodbye" || reason === "budget") && this.audio.playing) this.closing = reason;
					else this.end(reason);
					return;
				}
				this.desk = event.state;
				this.settle();
				return;
			case "heard":
				this.line({ kind: "you", id: `heard-${event.seq}`, text: event.text });
				return;
			case "delivery":
				this.pendingFrom = this.names(event.personaId) ?? "A teammate";
				return;
			case "said": {
				this.seen.add(event.id);
				const from = this.pendingFrom;
				this.pendingFrom = undefined;
				this.line({ kind: "desk", id: event.id, text: event.text, ...(from ? { from } : {}) });
				return;
			}
			case "clip":
				this.seen.add(event.id);
				if (this.held || this.muted.has(event.id)) return;
				this.audio.play(event.mimeType, event.data);
				this.settle();
				return;
			case "card":
				if (this.snapshot.cards.some((card) => card.requestId === event.requestId)) return;
				this.set({ cards: [...this.snapshot.cards, { personaId: event.personaId, requestId: event.requestId, kind: event.kind }] });
				return;
		}
	}

	/**
	 * Where the call stands, worked out again from what is true now: the
	 * line, the hold, what is playing, and what the desk last said. Every
	 * change goes through here, so no one event can leave the mic shut.
	 */
	settle(): void {
		if (this.ended) return;
		if (this.closing !== null && !this.audio.playing) return this.end(this.closing);
		// Until the desk has made the call and it is watched, it is still connecting.
		if (this.unsubscribe === null) return;
		if (this.held) return this.set({ phase: "held" });
		if (this.audio.playing) return this.set({ phase: "speaking" });
		if (this.awaiting || this.desk === "thinking" || this.desk === "speaking") return this.set({ phase: "thinking" });
		const phase = this.snapshot.phase;
		if (phase === "listening" || phase === "hearing") return;
		this.frames = [];
		this.heard = false;
		this.detector.reset(this.now());
		this.set({ phase: "listening" });
	}

	private connection(state: Reach): void {
		if (this.ended) return;
		if (state === "gone") return this.fail("This desk is no longer paired with this computer.");
		// The desk ends a call whose connection drops, so there is nothing to wait for.
		if (state === "closed") this.fail(LOST);
	}


	private async micLost(): Promise<void> {
		if (this.ended || this.held) return;
		try {
			await this.audio.reopenMic();
		} catch {
			this.fail("Hotline can't hear the microphone any more. Call again when it's back.");
		}
	}

	// ------------------------------------------------------------- hearing

	/** Exposed for tests; the microphone is the only caller in the window. */
	hear(block: Float32Array, rate: number): void {
		const phase = this.snapshot.phase;
		if (phase !== "listening" && phase !== "hearing") return;
		this.rate = rate;
		const level = rms(block);
		this.level = level;
		this.frames.push(block.slice());
		if (!this.heard) {
			const keep = Math.ceil(((PREROLL_MS / 1000) * rate) / block.length);
			while (this.frames.length > keep) this.frames.shift();
		}
		for (const turn of this.detector.push(level, this.now())) {
			switch (turn.kind) {
				case "start":
					this.heard = true;
					this.set({ phase: "hearing" });
					break;
				case "drop":
					this.frames = [];
					this.heard = false;
					break;
				case "end":
					this.send();
					return;
			}
		}
	}

	private send(): void {
		const rate = this.rate;
		const total = this.frames.reduce((sum, block) => sum + block.length, 0);
		const joined = new Float32Array(total);
		let at = 0;
		for (const block of this.frames) {
			joined.set(block, at);
			at += block.length;
		}
		this.frames = [];
		this.heard = false;
		const clip = encodeWav(downsample(joined, rate, WAV_RATE));
		const seq = ++this.seq;
		this.awaiting = true;
		this.settle();
		this.audio.chime("think");
		this.transport
			.command("voice.utterance", {
				callId: this.id,
				seq,
				mimeType: "audio/wav",
				data: toBase64(clip),
				durationMs: Math.round((total / rate) * 1000),
			})
			.then(() => {
				if (this.snapshot.trouble !== undefined) this.set({ trouble: undefined });
			})
			.catch((error: unknown) => {
				this.awaiting = false;
				this.set({ trouble: error instanceof Error ? error.message : String(error) });
				this.settle();
			});
	}

	// ------------------------------------------------------------- the rest

	private get ended(): boolean {
		return this.snapshot.phase === "ended";
	}

	private tick = (): void => {
		if (this.ended) return;
		let raw = 0;
		if (this.snapshot.phase === "speaking") raw = this.audio.outputLevel();
		else if (this.snapshot.phase === "listening" || this.snapshot.phase === "hearing") raw = this.level;
		// Perceptual curve, fast attack and slow release, as Spark drew it.
		const target = Math.min(1, Math.pow(raw * 9, 0.6));
		this.shown += (target - this.shown) * (target > this.shown ? 0.5 : 0.12);
		for (const listener of this.levels) listener(this.shown);
		this.raf = requestAnimationFrame(this.tick);
	};

	private pauseClock(): void {
		const { base, since } = this.snapshot.clock;
		if (since !== null) this.set({ clock: { base: base + (Date.now() - since), since: null } });
	}

	private resumeClock(): void {
		if (this.snapshot.clock.since === null) this.set({ clock: { base: this.snapshot.clock.base, since: Date.now() } });
	}

	private line(line: CallLine): void {
		const lines = this.snapshot.lines.filter((one) => one.id !== line.id);
		this.set({ lines: [...lines, line].slice(-60) });
	}

	private fail(trouble: string): void {
		this.set({ trouble });
		this.end("error");
	}

	private end(reason: EndReason): void {
		if (this.ended) return;
		this.pauseClock();
		const words = ENDED_WORDS[reason];
		this.set({ phase: "ended", ended: reason, ...(words && !this.snapshot.trouble ? { trouble: words } : {}) });
		cancelAnimationFrame(this.raf);
		for (const listener of this.levels) listener(0);
		this.unwatch?.();
		this.unwatch = null;
		this.unsubscribe?.();
		this.unsubscribe = null;
		this.audio.stopPlayback();
		const tone = this.audio.chime("end");
		setTimeout(() => this.audio.close(), tone * 1000 + 80);
	}

	private set(patch: Partial<CallSnapshot>): void {
		this.snapshot = { ...this.snapshot, ...patch };
		for (const listener of this.listeners) listener();
	}
}

// ---------------------------------------------------------------- the one call

let active: Call | null = null;
const holders = new Set<() => void>();

/** The window's call, if one is open or just ended. */
export function currentCall(): Call | null {
	return active;
}

export async function startCall(names?: Names): Promise<Call> {
	active?.hangUp();
	const call = new Call(deskTransport(), names);
	active = call;
	for (const holder of holders) holder();
	await call.start();
	return call;
}

/** Closes the pane: hangs up if it is still open. */
export function closeCall(): void {
	active?.hangUp();
	active = null;
	for (const holder of holders) holder();
}

export function useCall(): Call | null {
	return useSyncExternalStore(
		(listener) => {
			holders.add(listener);
			return () => holders.delete(listener);
		},
		() => active,
	);
}

const EMPTY: CallSnapshot = { phase: "ended", lines: [], cards: [], clock: { base: 0, since: null } };

export function useCallSnapshot(call: Call | null): CallSnapshot {
	return useSyncExternalStore(
		(listener) => call?.watch(listener) ?? (() => {}),
		() => call?.current ?? EMPTY,
	);
}

/**
 * Whether the open desk can take a call: it answers `voice.status` with a
 * speech provider it can use. A desk from before voice refuses the command,
 * and that is a no, not an error.
 */
export function useVoiceAvailable(connection: string): boolean {
	const [available, setAvailable] = useState(false);
	useEffect(() => {
		if (connection !== "open") return;
		let current = true;
		wireTransport
			.command("voice.status", {})
			.then((status) => {
				if (current) setAvailable((status as { available?: boolean } | null)?.available === true);
			})
			.catch(() => {
				if (current) setAvailable(false);
			});
		return () => {
			current = false;
		};
	}, [connection]);
	return available;
}
