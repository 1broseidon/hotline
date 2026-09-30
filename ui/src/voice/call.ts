import { useEffect, useState, useSyncExternalStore } from "react";
import { wire } from "../wire";
import { ClipPlayer } from "./player";
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

export type EndReason = "client" | "goodbye" | "budget" | "replaced" | "error" | "idle";

export type CallSnapshot = {
	phase: CallPhase;
	lines: CallLine[];
	cards: CallCard[];
	startedAt: number;
	/** Why it ended, once it has. */
	ended?: EndReason;
	/** One sentence for a person, when something went wrong. */
	trouble?: string;
};

/** What the desk sends on `{call: id}`. */
export type CallEvent =
	| { type: "state"; state: "listening" | "thinking" | "speaking" | "held" | "ended"; reason?: EndReason }
	| { type: "heard"; seq: number; text: string }
	| { type: "said"; id: string; text: string }
	| { type: "clip"; id: string; index: number; final: boolean; mimeType: string; data: string }
	| { type: "delivery"; personaId: string; eventId: string; text: string }
	| { type: "card"; personaId: string; requestId: string; kind: string };

/** The two things a call needs from a desk; the wire in the window, a fake in tests. */
export type CallTransport = {
	command(cmd: string, params: Record<string, unknown>): Promise<unknown>;
	subscribe(target: unknown, handlers: { snapshot(items: unknown[]): void; event(item: unknown): void }): () => void;
};

/* voice.* is not in the generated Command union until the desk core lands
 * it; the contract is BRO-168's, and this is the one place that spells it. */
const wireTransport: CallTransport = {
	command: (cmd, params) => (wire.command as (c: string, p: unknown) => Promise<unknown>)(cmd, params),
	subscribe: (target, handlers) =>
		(wire.subscribe as (t: unknown, h: unknown) => () => void)(target, handlers),
};

const ENDED_WORDS: Record<EndReason, string | undefined> = {
	client: undefined,
	goodbye: undefined,
	idle: "The call ended after ten quiet minutes.",
	budget: "Today's voice budget is spent. Carry on by text.",
	replaced: "The call moved to another device.",
	error: "The desk dropped the call.",
};

/** How much audio before the detector is sure it heard speech is kept, so a word's first sound is not clipped. */
const PREROLL_MS = 400;
/** The mic is read in blocks of this many frames: ~43ms at 48 kHz. */
const BLOCK = 2048;

type Names = (personaId: string) => string | undefined;

export class Call {
	readonly id = crypto.randomUUID();
	private snapshot: CallSnapshot = { phase: "connecting", lines: [], cards: [], startedAt: Date.now() };
	private readonly listeners = new Set<() => void>();
	private readonly levels = new Set<(level: number) => void>();

	private ctx: AudioContext | null = null;
	private stream: MediaStream | null = null;
	private processor: ScriptProcessorNode | null = null;
	private outAnalyser: AnalyserNode | null = null;
	private player: ClipPlayer | null = null;
	private detector = new TurnDetector(performance.now());
	private frames: Float32Array[] = [];
	private heard = false;
	private seq = 0;
	private unsubscribe: (() => void) | null = null;
	private deskPhase: CallEvent & { type: "state" } = { type: "state", state: "listening" };
	private pendingFrom: string | undefined;
	private level = 0;
	private raf = 0;

	constructor(
		private readonly transport: CallTransport = wireTransport,
		private readonly names: Names = () => undefined,
	) {}

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
		const ctx = new AudioContext();
		this.ctx = ctx;
		await ctx.resume();
		try {
			this.stream = await navigator.mediaDevices.getUserMedia({
				audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1 },
			});
		} catch {
			this.fail("Hotline can't hear the microphone. Allow it in your system settings, then call again.");
			return;
		}
		if (this.snapshot.phase === "ended") return this.release();

		const out = ctx.createGain();
		const analyser = ctx.createAnalyser();
		analyser.fftSize = 1024;
		out.connect(analyser);
		analyser.connect(ctx.destination);
		this.outAnalyser = analyser;
		this.player = new ClipPlayer(ctx, out, () => this.spoken());

		const mic = ctx.createMediaStreamSource(this.stream);
		const processor = ctx.createScriptProcessor(BLOCK, 1, 1);
		processor.onaudioprocess = (event) => this.hear(event.inputBuffer.getChannelData(0));
		mic.connect(processor);
		// A script processor only runs while it leads somewhere; this gain is silent.
		const sink = ctx.createGain();
		sink.gain.value = 0;
		processor.connect(sink);
		sink.connect(ctx.destination);
		this.processor = processor;

		this.unsubscribe = this.transport.subscribe(
			{ call: this.id },
			{
				snapshot: (items) => {
					for (const item of items) this.receive(item as CallEvent);
				},
				event: (item) => this.receive(item as CallEvent),
			},
		);
		try {
			await this.transport.command("voice.call_start", { callId: this.id });
		} catch (error) {
			this.fail(error instanceof Error ? error.message : String(error));
			return;
		}
		this.listen();
		this.chime("connect");
		this.tick();
	}

	hangUp(): void {
		if (this.snapshot.phase === "ended") return;
		void this.transport.command("voice.call_end", { callId: this.id }).catch(() => {});
		this.end("client");
	}

	hold(on: boolean): void {
		const phase = this.snapshot.phase;
		if (phase === "ended" || phase === "connecting" || on === (phase === "held")) return;
		void this.transport.command("voice.hold", { callId: this.id, hold: on }).catch(() => {});
		this.stream?.getAudioTracks().forEach((track) => (track.enabled = !on));
		if (on) {
			this.player?.stop();
			this.set({ phase: "held" });
		} else {
			this.listen();
		}
	}

	/** Cuts the desk off mid-sentence. It never stops a teammate's turn. */
	interrupt(): void {
		if (this.snapshot.phase !== "speaking" && this.snapshot.phase !== "thinking") return;
		void this.transport.command("voice.interrupt", { callId: this.id }).catch(() => {});
		this.player?.stop();
		this.listen();
	}

	dismissCard(requestId: string): void {
		this.set({ cards: this.snapshot.cards.filter((card) => card.requestId !== requestId) });
	}

	// ------------------------------------------------------------- the desk

	private receive(event: CallEvent): void {
		if (this.snapshot.phase === "ended") return;
		switch (event.type) {
			case "state":
				this.deskPhase = event;
				if (event.state === "ended") this.end(event.reason ?? "error");
				else if (event.state === "thinking" && this.snapshot.phase !== "held") this.set({ phase: "thinking" });
				return;
			case "heard":
				this.line({ kind: "you", id: `heard-${event.seq}`, text: event.text });
				return;
			case "delivery":
				this.pendingFrom = this.names(event.personaId) ?? "A teammate";
				return;
			case "said": {
				const from = this.pendingFrom;
				this.pendingFrom = undefined;
				this.line({ kind: "desk", id: event.id, text: event.text, ...(from ? { from } : {}) });
				return;
			}
			case "clip":
				if (this.snapshot.phase === "held") return;
				this.player?.push(event.mimeType, event.data);
				this.set({ phase: "speaking" });
				return;
			case "card":
				if (this.snapshot.cards.some((card) => card.requestId === event.requestId)) return;
				this.set({ cards: [...this.snapshot.cards, { personaId: event.personaId, requestId: event.requestId, kind: event.kind }] });
				return;
		}
	}

	/** The last clip finished: back to listening, unless the desk is still working on an answer. */
	private spoken(): void {
		if (this.snapshot.phase !== "speaking") return;
		if (this.deskPhase.state === "thinking") this.set({ phase: "thinking" });
		else this.listen();
	}

	// ------------------------------------------------------------- hearing

	private listen(): void {
		if (this.snapshot.phase === "ended") return;
		this.frames = [];
		this.heard = false;
		this.detector.reset(performance.now());
		this.set({ phase: "listening" });
	}

	private hear(block: Float32Array): void {
		const phase = this.snapshot.phase;
		if (phase !== "listening" && phase !== "hearing") return;
		const level = rms(block);
		this.level = level;
		this.frames.push(block.slice());
		if (!this.heard) {
			const keep = Math.ceil((PREROLL_MS / 1000) * (this.ctx?.sampleRate ?? 48_000) / BLOCK);
			while (this.frames.length > keep) this.frames.shift();
		}
		const turn = this.detector.push(level, performance.now());
		if (!turn) return;
		switch (turn.kind) {
			case "hearing":
				this.heard = true;
				this.set({ phase: "hearing" });
				return;
			case "listening":
				this.heard = false;
				this.set({ phase: "listening" });
				return;
			case "reset":
				this.frames = [];
				this.heard = false;
				return;
			case "end":
				this.send();
				return;
		}
	}

	private send(): void {
		const rate = this.ctx?.sampleRate ?? 48_000;
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
		this.set({ phase: "thinking" });
		this.chime("think");
		this.transport
			.command("voice.utterance", {
				callId: this.id,
				seq,
				mimeType: "audio/wav",
				data: toBase64(clip),
				durationMs: Math.round((total / rate) * 1000),
			})
			.catch((error: unknown) => {
				this.set({ trouble: error instanceof Error ? error.message : String(error) });
				this.listen();
			});
	}

	// ------------------------------------------------------------- the rest

	private tick = (): void => {
		if (this.snapshot.phase === "ended") return;
		let raw = 0;
		if (this.snapshot.phase === "speaking" && this.outAnalyser) {
			const buffer = new Float32Array(this.outAnalyser.fftSize);
			this.outAnalyser.getFloatTimeDomainData(buffer);
			raw = rms(buffer);
		} else if (this.snapshot.phase === "listening" || this.snapshot.phase === "hearing") {
			raw = this.level;
		}
		// Perceptual curve, fast attack and slow release, as Spark drew it.
		const target = Math.min(1, Math.pow(raw * 9, 0.6));
		const shown = (this.shown += (target - this.shown) * (target > this.shown ? 0.5 : 0.12));
		for (const listener of this.levels) listener(shown);
		this.raf = requestAnimationFrame(this.tick);
	};
	private shown = 0;

	private line(line: CallLine): void {
		const lines = this.snapshot.lines.filter((one) => one.id !== line.id);
		this.set({ lines: [...lines, line].slice(-60) });
	}

	private fail(trouble: string): void {
		this.set({ trouble });
		this.end("error");
	}

	private end(reason: EndReason): void {
		if (this.snapshot.phase === "ended") return;
		const words = ENDED_WORDS[reason];
		this.set({ phase: "ended", ended: reason, ...(words && !this.snapshot.trouble ? { trouble: words } : {}) });
		cancelAnimationFrame(this.raf);
		for (const listener of this.levels) listener(0);
		this.unsubscribe?.();
		this.unsubscribe = null;
		this.player?.stop();
		const tone = this.ctx?.state === "running" ? this.chime("end") : 0;
		setTimeout(() => this.release(), tone * 1000 + 80);
	}

	private release(): void {
		if (this.processor) this.processor.onaudioprocess = null;
		this.processor?.disconnect();
		this.stream?.getTracks().forEach((track) => track.stop());
		this.stream = null;
		void this.ctx?.close().catch(() => {});
		this.ctx = null;
	}

	private set(patch: Partial<CallSnapshot>): void {
		this.snapshot = { ...this.snapshot, ...patch };
		for (const listener of this.listeners) listener();
	}

	/** Soft sine blips, like a phone line: rising to connect, one tick to think, falling to hang up. Seconds long. */
	private chime(kind: "connect" | "think" | "end"): number {
		const ctx = this.ctx;
		if (!ctx || ctx.state === "closed") return 0;
		const notes = { connect: [587.33, 880], think: [1174.66], end: [880, 587.33] }[kind];
		const peak = kind === "think" ? 0.035 : 0.06;
		const length = kind === "think" ? 0.07 : 0.11;
		const step = 0.09;
		const t0 = ctx.currentTime + 0.02;
		notes.forEach((frequency, i) => {
			const at = t0 + i * step;
			const osc = ctx.createOscillator();
			const gain = ctx.createGain();
			osc.type = "sine";
			osc.frequency.value = frequency;
			gain.gain.setValueAtTime(0, at);
			gain.gain.linearRampToValueAtTime(peak, at + 0.008);
			gain.gain.exponentialRampToValueAtTime(0.0001, at + length);
			osc.connect(gain).connect(ctx.destination);
			osc.start(at);
			osc.stop(at + length + 0.02);
		});
		return 0.02 + (notes.length - 1) * step + length;
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
	const call = new Call(wireTransport, names);
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

const EMPTY: CallSnapshot = { phase: "ended", lines: [], cards: [], startedAt: 0 };

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
