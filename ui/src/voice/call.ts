import { useEffect, useState, useSyncExternalStore } from "react";
import { activeDeskId, allDesks, watchDesks, wireFor } from "../desks";
import { type Target, wire } from "../wire";
import type { FileChunk, VoiceEndReason, VoiceEvent } from "../generated/contract";
import { type CallAudio, webAudio } from "./audio";
import { TurnDetector, levelFromDb } from "./turn";
import { type DeviceTranscription, type TranscriptionEvent, deviceTranscription, hearsOnThisMac } from "./transcription";
import { WAV_RATE, downsample, encodeWav, rms, toBase64 } from "./wav";
import { PcmTurn } from "./stream";
import { useRawSetting } from "../room";

/**
 * A call belongs to its chosen desk and optional teammate. It lives outside
 * React so switching conversations or desks does not retarget the call.
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
function deskTransport(deskId: string | null): CallTransport {
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

const NO_MIC = "Hotline can't hear the microphone. Allow it in your system settings, then call again.";

const NO_TRANSCRIPTION = "Speech recognition on this Mac could not start. Check speech access in your system settings, then call again.";

/** How much audio before the detector is sure it heard speech is kept, so a word's first sound is not clipped. */
const PREROLL_MS = 400;

/** How often the blip-blip repeats while the desk works and says nothing. */
const WORKING_EVERY_MS = 1800;

/** Recognized words corroborate a short answer such as "yes", so a text call's onset can be shorter than a clip's. */
const TEXT_MIN_SPEECH_MS = 100;

/** Words recognized this long before the speaker's onset were the room's, not theirs. */
const TEXT_ONSET_PREROLL_MS = 500;

type Names = (personaId: string) => string | undefined;

export type CallTarget = { personaId: string; name: string; avatarHash?: string | undefined };
export type CallOptions = { deskId?: string | null; target?: CallTarget | undefined };

/** Remove only a stable recognized prefix known to predate the speaker's onset. */
function afterRecognizedPrefix(text: string, prefix: string): string {
	if (prefix === "") return text.trim();
	const words = text.trim().split(/\s+/);
	const before = prefix.trim().split(/\s+/);
	const spoken = (word: string) => word.toLowerCase().replace(/[\p{P}\p{S}]/gu, "");
	if (before.some((word, index) => spoken(word) !== spoken(words[index] ?? ""))) return text.trim();
	return words.slice(before.length).join(" ");
}

export function supportsDirectCalls(status: unknown): boolean {
	if (typeof status !== "object" || status === null) return false;
	const value = status as { capabilities?: unknown; directAvailable?: unknown; available?: unknown };
	return Array.isArray(value.capabilities) && value.capabilities.includes("voiceDirectCalls") &&
		(value.directAvailable ?? value.available) === true;
}

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
	private pcm = false;
	private input: PcmTurn | null = null;
	private endpoint: { at: number; seq: number } | null = null;
	/** A progressive output line can temporarily drain before its final chunk arrives. */
	private readonly outputLines = new Set<string>();
	private holdGeneration = 0;
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
	private readonly clipIndices = new Map<string, number>();
	private pendingFrom: string | undefined;
	/** The desk has ended the call; this waits for the last clip to finish first. */
	private closing: EndReason | null = null;
	private level = 0;
	private shown = 0;
	private raf = 0;
	/** While the desk works, a blip-blip every so often says it still is. */
	private working: ReturnType<typeof setInterval> | null = null;

	/**
	 * A text call: this Mac recognizes the speech and the desk gets finished
	 * words (`voice.text`). The engine has the microphone, one utterance per
	 * session, and only while the call listens, so it never hears the desk.
	 */
	private textInput = false;
	/** Which session's events count; bumped whenever one is let go. */
	private textSession = 0;
	/** A session is open and its words are the live line. */
	private recognizing = false;
	/** Words from earlier sessions of this turn, when the engine ended one on its own. */
	private textPrefix = "";
	/** The open session's words so far: each event is the whole of them. */
	private textPart = "";
	/** The session's words from before the speaker's onset, which are the room's. */
	private textIgnored = "";
	private textSnapshots: { text: string; at: number }[] = [];
	/** The connect chime is not the speaker. */
	private quietUntil = 0;

	constructor(
		private readonly transport: CallTransport = wireTransport,
		private readonly names: Names = () => undefined,
		audio?: CallAudio,
		private readonly now: () => number = () => performance.now(),
		private readonly options: CallOptions = {},
		private readonly transcription: DeviceTranscription | undefined = deviceTranscription(),
	) {
		this.audio = audio ?? webAudio({ onIdle: () => this.settle(), onLost: () => void this.micLost(), onStarted: () => this.audible() });
		this.detector = new TurnDetector(this.now());
	}

	// ------------------------------------------------------------- reading

	get current(): CallSnapshot {
		return this.snapshot;
	}

	get target(): CallTarget | undefined { return this.options.target; }
	get deskId(): string | null | undefined { return this.options.deskId; }
	nameOf(personaId: string): string | undefined { return this.names(personaId); }
	readonly readAvatar = (personaId: string, hash: string, offset: number): Promise<FileChunk> =>
		this.transport.command("avatar.read", { personaId, hash, offset }) as Promise<FileChunk>;
	redial(): Call { return new Call(this.transport, this.names, undefined, undefined, this.options, this.transcription); }

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
			this.fail(NO_MIC);
			return;
		}
		if (this.ended) return;
		try {
			let status: unknown = null;
			// Where this Mac can hear the words, the desk is asked about a text call: it then needs
			// a voice, not a transcription provider of its own.
			const asked = this.transcription !== undefined && (await this.transcription.capability()).available ? { inputMode: "text" as const } : {};
			if (this.ended) return;
			if (this.target !== undefined) status = await this.transport.command("voice.status", asked);
			// A desk call asks only to learn whether the desk takes text; not knowing keeps it on audio.
			else if (this.transcription !== undefined) status = await this.transport.command("voice.status", asked).catch(() => null);
			if (this.ended) return;
			if (this.target !== undefined && !supportsDirectCalls(status)) throw new Error(
				(status as { unavailable?: string } | null)?.unavailable ?? "This desk needs an update before it can call a teammate directly.",
			);
			this.textInput = await this.transcribes(status);
			if (this.ended) return;
		} catch (error) {
			this.fail(error instanceof Error ? error.message : String(error));
			return;
		}
		if (this.textInput) this.detector = new TurnDetector(this.now(), TEXT_MIN_SPEECH_MS);
		else {
			try {
				await this.audio.openMic();
			} catch {
				this.fail(NO_MIC);
				return;
			}
			if (this.ended) return;
		}
		// The call exists once the desk has answered call_start; only then can it be watched.
		try {
			const response = await this.transport.command("voice.call_start", {
				callId: this.id,
				streamAudio: true,
				...(this.target !== undefined ? { personaId: this.target.personaId } : {}),
				...(this.textInput ? { inputMode: "text" } : {}),
			}) as { input?: string[]; personaId?: string } | null;
			this.started = true;
			if (this.target !== undefined && response?.personaId !== this.target.personaId) {
				throw new Error("The desk did not connect the call to the chosen teammate.");
			}
			if (this.textInput && response?.input?.includes("text/plain") !== true) {
				throw new Error("The desk did not accept on-device transcription. Update it and call again.");
			}
			this.pcm = !this.textInput && response?.input?.includes("audio/pcm") === true;
		} catch (error) {
			this.fail(error instanceof Error ? error.message : String(error));
			return;
		}
		if (this.ended) {
			// Hung up while the desk was answering: tell it, now that it knows the call.
			void this.transport.command("voice.call_end", { callId: this.id }).catch(() => {});
			return;
		}
		const unsubscribe = this.transport.subscribe(
			{ call: this.id },
			{
				// A snapshot is where the call stands, never audio to play again.
				snapshot: (items) => {
					for (const item of items) if ((item as CallEvent).type === "state") this.receive(item as CallEvent);
				},
				event: (item) => this.receive(item as CallEvent),
			},
		);
		if (this.ended) { unsubscribe(); return; }
		this.unsubscribe = unsubscribe;
		const unwatch = this.transport.onConnection?.((state) => this.connection(state)) ?? null;
		if (this.ended) { unwatch?.(); return; }
		this.unwatch = unwatch;
		this.set({ clock: { base: 0, since: Date.now() } });
		const tone = this.audio.chime("connect");
		this.quietUntil = this.now() + tone * 1000 + 120;
		this.settle();
		this.tick();
	}

	/**
	 * Whether this call hears through this Mac's own speech recognition:
	 * the desk takes text, the engine is here, and the person allows it.
	 * Anything short of that keeps the call on audio, as before.
	 */
	private async transcribes(status: unknown): Promise<boolean> {
		const transcription = this.transcription;
		if (transcription === undefined) return false;
		const capabilities = (status as { capabilities?: unknown } | null)?.capabilities;
		if (!Array.isArray(capabilities) || !capabilities.includes("voiceTextInput")) return false;
		const capability = await transcription.capability();
		if (!capability.available || this.ended) return false;
		return transcription.permit();
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
			this.holdGeneration++;
			this.held = true;
			this.cancelInput();
			this.endpoint = null;
			this.awaiting = false;
			this.outputLines.clear();
			for (const id of this.seen) this.muted.add(id);
			this.audio.stopPlayback();
			// Let the microphone go, so the system's mic light says so too.
			this.audio.closeMic();
			this.pauseClock();
			this.settle();
			return;
		}
		const generation = ++this.holdGeneration;
		// A text call's engine takes the microphone again when the call listens.
		if (!this.textInput) {
			try {
				await this.audio.openMic();
			} catch {
				this.fail("Hotline can't hear the microphone any more. Call again when it's back.");
				return;
			}
		}
		if (this.ended || generation !== this.holdGeneration) return;
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
		this.outputLines.clear();
		this.endpoint = null;
		this.audio.stopPlayback();
		this.awaiting = false;
		// Work the desk has not finished keeps it thinking; it says when it can listen.
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
				// The producer is finished at these authoritative states, including a failed partial TTS stream.
				if (event.state === "listening" || event.state === "ended") this.outputLines.clear();
				if (event.state === "ended") {
					const reason = event.reason ?? "error";
					// A goodbye, a spent budget or a failure is said before the line goes: let the last sentence finish.
					if ((reason === "goodbye" || reason === "budget" || reason === "error") && (this.audio.playing || this.outputLines.size > 0)) this.closing = reason;
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
				this.remember(event.id);
				const from = this.pendingFrom;
				this.pendingFrom = undefined;
				this.line({ kind: "desk", id: event.id, text: event.text, ...(from ? { from } : {}) });
				return;
			}
			case "clip":
				if (!this.seen.has(event.id) && event.index !== 0) return;
				this.remember(event.id);
				if (event.index !== (this.clipIndices.get(event.id) ?? 0)) return;
				this.clipIndices.set(event.id, event.index + 1);
				if (this.held || this.muted.has(event.id)) return;
				if (event.final) this.outputLines.delete(event.id);
				else this.outputLines.add(event.id);
				try { if (event.data !== "") this.audio.play(event.mimeType, event.data); }
				catch (error) { this.fail(error instanceof Error ? error.message : String(error)); return; }
				this.settle();
				return;
			case "card":
				if (this.snapshot.cards.some((card) => card.requestId === event.requestId)) return;
				this.set({ cards: [...this.snapshot.cards, { personaId: event.personaId, requestId: event.requestId, kind: event.kind }] });
				return;
		}
	}

	private remember(id: string): void {
		this.seen.add(id);
		while (this.seen.size > 128) {
			const oldest = this.seen.values().next().value!;
			this.seen.delete(oldest);
			this.muted.delete(oldest);
			this.clipIndices.delete(oldest);
			this.outputLines.delete(oldest);
		}
	}

	/**
	 * Where the call stands, worked out again from what is true now: the
	 * line, the hold, what is playing, and what the desk last said. Every
	 * change goes through here, so no one event can leave the mic shut.
	 */
	settle(): void {
		if (this.ended) return;
		if (this.closing !== null && !this.audio.playing && this.outputLines.size === 0) return this.end(this.closing);
		// Until the desk has made the call and it is watched, it is still connecting.
		if (this.unsubscribe === null) return;
		if (this.held) return this.set({ phase: "held" });
		if (this.audio.playing) return this.set({ phase: "speaking" });
		if (this.awaiting || this.outputLines.size > 0 || this.desk === "thinking" || this.desk === "speaking") return this.set({ phase: "thinking" });
		const phase = this.snapshot.phase;
		if (phase === "listening" || phase === "hearing") return;
		this.cancelInput();
		this.detector.reset(this.now());
		this.set({ phase: "listening" });
		if (this.textInput) this.listen();
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
			await this.audio.openMic();
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
		if (this.input !== null) {
			if (this.input.push(block)) { this.send(); return; }
			if (this.ended) return;
		} else this.frames.push(block.slice());
		if (!this.heard) {
			const keep = Math.ceil(((PREROLL_MS / 1000) * rate) / block.length);
			while (this.frames.length > keep) this.frames.shift();
		}
		for (const turn of this.detector.push(level, this.now())) {
			switch (turn.kind) {
				case "start":
					this.heard = true;
					this.set({ phase: "hearing" });
					if (this.pcm) {
						const seq = ++this.seq;
						this.input = new PcmTurn(rate,
							(chunk) => this.transport.command("voice.audio", { callId: this.id, seq, ...chunk }),
							(error) => this.streamFailed(error));
						for (const frame of this.frames) this.input.push(frame);
						this.frames = [];
					}
					break;
				case "drop":
					// Silence, or a steady noise taken for speech until the floor caught up: nothing to send.
					this.cancelInput();
					if (this.snapshot.phase === "hearing") this.set({ phase: "listening" });
					break;
				case "end":
					this.send();
					return;
			}
		}
		if (this.heard && !this.pcm && this.frames.reduce((sum, frame) => sum + frame.length, 0) >= rate * 20) this.send();
	}

	private send(): void {
		this.endpoint = { at: this.now(), seq: this.input !== null ? this.seq : this.seq + 1 };
		if (this.input !== null) {
			const input = this.input;
			this.awaiting = true;
			this.settle();
			this.audio.chime("think");
			void input.finish().then(() => {
				if (this.input === input) this.input = null;
				if (!this.ended && this.snapshot.trouble !== undefined) this.set({ trouble: undefined });
			}).catch(() => {}); // Transport errors end the call through streamFailed; cancellation is silent.
			return;
		}
		const rate = this.rate;
		const total = Math.min(this.frames.reduce((sum, block) => sum + block.length, 0), rate * 20);
		const joined = new Float32Array(total);
		let at = 0;
		for (const block of this.frames) {
			const slice = block.subarray(0, total - at);
			joined.set(slice, at);
			at += slice.length;
			if (at === total) break;
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
			.then(() => this.delivered(), (error: unknown) => this.refused(error));
	}

	private delivered(): void {
		if (this.snapshot.trouble !== undefined) this.set({ trouble: undefined });
	}

	/** The desk turned an utterance down: say why, and listen again. */
	private refused(error: unknown): void {
		if (this.ended || this.held) return;
		this.endpoint = null;
		this.awaiting = false;
		this.set({ trouble: error instanceof Error ? error.message : String(error) });
		this.settle();
	}

	private cancelInput(): void {
		this.input?.cancel();
		this.input = null;
		this.frames = [];
		this.heard = false;
		if (this.textInput) this.stopListening();
	}

	// ------------------------------------------------------------- hearing on this Mac

	/** A fresh recognition session; the words of a turn the engine split are kept. */
	private listen(): void {
		const session = ++this.textSession;
		this.recognizing = true;
		this.textPart = "";
		this.textIgnored = "";
		this.textSnapshots = [];
		const failed = () => {
			if (session === this.textSession && !this.ended) this.fail(NO_TRANSCRIPTION);
		};
		this.transcription!.start((event) => {
			if (session === this.textSession && this.recognizing && !this.ended) this.transcribed(event);
		}).then((started) => {
			if (!started) failed();
		}, failed);
	}

	/** Lets the session go, unsent: the call holds, the desk has the floor, or nobody spoke. */
	private stopListening(): void {
		this.textSession++;
		this.recognizing = false;
		this.textPrefix = "";
		this.textPart = "";
		this.textIgnored = "";
		this.textSnapshots = [];
		this.level = 0;
		this.removeLine(`heard-${this.seq + 1}`);
		void this.transcription?.cancel().catch(() => {});
	}

	private transcribed(event: TranscriptionEvent): void {
		switch (event.type) {
			case "level": {
				const now = this.now();
				if (now < this.quietUntil) return;
				const level = levelFromDb(event.levelDb);
				this.level = level;
				for (const turn of this.detector.push(level, now)) {
					switch (turn.kind) {
						case "start":
							this.heard = true;
							// Words the engine had before the onset were the room's, not the speaker's.
							for (const snapshot of this.textSnapshots) {
								if (snapshot.at < turn.at - TEXT_ONSET_PREROLL_MS) this.textIgnored = snapshot.text;
							}
							this.textSnapshots = [];
							this.set({ phase: "hearing" });
							this.showDraft();
							break;
						case "drop":
							// Nobody spoke: a fresh session, so the room's noise never piles up in one.
							this.cancelInput();
							this.listen();
							return;
						case "end":
							void this.sendText();
							return;
					}
				}
				return;
			}
			case "partial":
			case "final": {
				const text = event.text.trim();
				if (!this.heard) {
					this.textSnapshots.push({ text, at: this.now() });
					if (this.textSnapshots.length > 64) this.textSnapshots.shift();
				}
				this.textPart = text;
				this.showDraft();
				return;
			}
			case "error":
				this.fail(event.message || NO_TRANSCRIPTION);
				return;
			case "ended":
				if (event.reason === "error") return this.fail("Speech recognition on this Mac stopped. Call again.");
				if (event.reason === "cancelled") return;
				// The engine ended a session on its own: keep its words and listen on, so a pause does not split the turn.
				this.textPrefix = this.spoken(this.textPart);
				this.listen();
				return;
		}
	}

	/** What the speaker has said this turn: nothing until the meter heard a voice, so the room's words are never sent. */
	private spoken(part: string): string {
		if (!this.heard) return "";
		return [this.textPrefix, afterRecognizedPrefix(part, this.textIgnored)].filter((one) => one !== "").join(" ").trim();
	}

	/** The words so far are the live line, under the id the desk's own `heard` will replace. */
	private showDraft(): void {
		const text = this.spoken(this.textPart);
		if (text === "") this.removeLine(`heard-${this.seq + 1}`);
		else this.line({ kind: "you", id: `heard-${this.seq + 1}`, text });
	}

	/** The turn has ended: the engine finishes the utterance, and its complete words go to the desk. */
	private async sendText(): Promise<void> {
		const session = this.textSession;
		this.recognizing = false;
		this.endpoint = { at: this.now(), seq: this.seq + 1 };
		this.awaiting = true;
		this.settle();
		let text: string;
		try {
			text = this.spoken(await this.transcription!.stop());
		} catch (error) {
			if (session === this.textSession && !this.ended && !this.held) this.fail(error instanceof Error ? error.message : String(error));
			return;
		}
		// Held, cut in on or hung up while the engine finished: those words are not sent.
		if (session !== this.textSession || this.ended || this.held) return;
		if (text === "") {
			// A cough or a tap: nothing to say.
			this.endpoint = null;
			this.awaiting = false;
			this.settle();
			return;
		}
		const seq = ++this.seq;
		this.line({ kind: "you", id: `heard-${seq}`, text });
		// After the engine let the microphone go, so the blip-blip is never transcribed.
		this.audio.chime("think");
		this.transport
			.command("voice.text", { callId: this.id, seq, text })
			.then(() => this.delivered(), (error: unknown) => this.refused(error));
	}

	private streamFailed(error: unknown): void {
		if (this.ended || this.held) return;
		this.fail(error instanceof Error ? error.message : String(error));
	}

	private audible(): void {
		if (this.endpoint === null || this.ended || this.held) return;
		const { at, seq } = this.endpoint;
		this.endpoint = null;
		console.info("Hotline voice endpoint to audible start", { seq, durationMs: Math.round(this.now() - at) });
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

	private removeLine(id: string): void {
		if (this.snapshot.lines.some((one) => one.id === id)) this.set({ lines: this.snapshot.lines.filter((one) => one.id !== id) });
	}

	private fail(trouble: string): void {
		if (this.ended) return;
		if (this.started) void this.transport.command("voice.call_end", { callId: this.id }).catch(() => {});
		this.set({ trouble });
		this.end("error");
	}

	private end(reason: EndReason): void {
		if (this.ended) return;
		this.holdGeneration++;
		this.cancelInput();
		// Before the call settles on text, this stops a language download that asking permission started.
		if (!this.textInput) void this.transcription?.cancel().catch(() => {});
		this.outputLines.clear();
		this.endpoint = null;
		this.audio.closeMic();
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
		// The desk has the floor or the call is held: the engine lets the microphone go, so it never hears the desk.
		if (this.recognizing && this.snapshot.phase !== "listening" && this.snapshot.phase !== "hearing") this.stopListening();
		// The first blip-blip goes with the utterance; these repeat it while the desk is quiet and busy.
		if (this.snapshot.phase === "thinking" && this.working === null) {
			this.working = setInterval(() => this.audio.chime("think"), WORKING_EVERY_MS);
		} else if (this.snapshot.phase !== "thinking" && this.working !== null) {
			clearInterval(this.working);
			this.working = null;
		}
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

export async function startCall(names?: Names, target?: CallTarget): Promise<Call> {
	active?.hangUp();
	const deskId = activeDeskId();
	const call = new Call(deskTransport(deskId), names, undefined, undefined, { deskId, target });
	active = call;
	for (const holder of holders) holder();
	await call.start();
	return call;
}

/** Retry the original desk and target, even after the window moved elsewhere. */
export async function restartCall(previous: Call): Promise<Call> {
	active?.hangUp();
	const call = previous.redial();
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

const supportChecks = new Set<() => void>();

/**
 * Ask every open voice check to look again. Connecting or removing a
 * provider changes whether the desk can take a call, and the desk does not
 * announce it, so whoever changed one says so.
 */
export function recheckVoiceSupport(): void {
	for (const check of supportChecks) check();
}

/**
 * Whether the open desk can take a call: it answers `voice.status` with a
 * speech provider it can use. A desk from before voice refuses the command,
 * and that is a no, not an error. Asked again when a provider changes here,
 * when the voice settings change, and when the window comes back, which is
 * when a change made from the phone shows.
 */
export function useVoiceSupport(connection: string): { available: boolean; directCalls: boolean } {
	const [support, setSupport] = useState({ available: false, directCalls: false });
	const [asked, setAsked] = useState(0);
	const voiceSettings = useRawSetting("voice");
	useEffect(() => {
		const again = () => setAsked((n) => n + 1);
		supportChecks.add(again);
		window.addEventListener("focus", again);
		return () => {
			supportChecks.delete(again);
			window.removeEventListener("focus", again);
		};
	}, []);
	useEffect(() => {
		if (connection !== "open") { setSupport({ available: false, directCalls: false }); return; }
		let current = true;
		// A Mac that hears on its own asks about a text call, which needs no transcription provider.
		void hearsOnThisMac()
			.then((here) => wireTransport.command("voice.status", here ? { inputMode: "text" } : {}))
			.then((status) => {
				if (current) setSupport({ available: (status as { available?: boolean } | null)?.available === true, directCalls: supportsDirectCalls(status) });
			})
			.catch(() => {
				if (current) setSupport({ available: false, directCalls: false });
			});
		return () => {
			current = false;
		};
	}, [connection, asked, voiceSettings]);
	return support;
}
