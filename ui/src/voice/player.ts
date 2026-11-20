import { fromBase64 } from "./wav";

/** Clips still waiting to be decoded; ones already scheduled to play are bounded by bytes. */
export const MAX_QUEUED_CLIPS = 128;
/**
 * Encoded audio decoded or scheduled and not yet heard. A Turn by turn reply streams
 * as many small WAV chunks far faster than it is spoken; at 24 kHz this is
 * about fifteen minutes, matching how far ahead a live voice may queue.
 */
const MAX_QUEUED_BYTES = 48 * 1024 * 1024;
/** A voice front's first frame waits this long, so network jitter between frames never leaves a gap. */
const PCM_LEAD_S = 0.08;
/**
 * How far ahead of the clock a front's frames may be queued. A live voice
 * streams a long answer's audio far faster than it is spoken, so a two-minute
 * story arrives as two minutes queued ahead; at 24 kHz that is under 6 MB.
 * Past this, something is wrong with the desk, not the answer.
 */
const MAX_PCM_AHEAD_S = 15 * 60;

/** Which clip of which `said` line a Turn by turn clip is. */
export type ClipId = { line: string; index: number };

/**
 * Where playback stands: the last clip that began to play, how far into it
 * the audio clock is, and its length, in ms. The clock stops while the call
 * is paused, so a reply talked over is placed where it paused.
 */
export type ClipPlayed = ClipId & { ms: number; clipMs: number };

/** Decode ahead and schedule consecutive clips on the audio clock, including progressive WAV chunks. */
export class ClipPlayer {
	private readonly sources = new Set<AudioBufferSourceNode>();
	private queue: Promise<{ buffer: AudioBuffer; bytes: number; clip: ClipId | undefined } | null>[] = [];
	/** Clips scheduled since playback last stopped: those still to begin, and the last that began. */
	private scheduled: (ClipId & { at: number; duration: number })[] = [];
	private decoding = false;
	private nextAt = 0;
	private generation = 0;
	private queuedBytes = 0;
	/** Where the speech playing now began on the audio clock; null while nothing plays. */
	private spokenAt: number | null = null;
	private readonly startTimers = new Set<ReturnType<typeof setTimeout>>();

	constructor(
		private readonly ctx: AudioContext,
		private readonly out: AudioNode,
		private readonly onIdle: () => void,
		private readonly onStarted: () => void = () => {},
	) {}

	get busy(): boolean {
		return this.decoding || this.queue.length > 0 || this.sources.size > 0;
	}

	push(mimeType: string, data: string, clip?: ClipId): void {
		if (this.queue.length + Number(this.decoding) >= MAX_QUEUED_CLIPS || data.length > MAX_QUEUED_BYTES * 4 / 3 + 4) {
			throw new Error("The desk sent more voice audio than Hotline can play. Call again.");
		}
		const bytes = fromBase64(data);
		if (this.queuedBytes + bytes.length > MAX_QUEUED_BYTES) throw new Error("The desk sent more voice audio than Hotline can play. Call again.");
		this.queuedBytes += bytes.length;
		const generation = this.generation;
		const decoding = this.ctx
			.decodeAudioData(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer)
			.catch((error: unknown) => {
				if (generation === this.generation) this.queuedBytes -= bytes.length;
				console.error(`Hotline could not play a ${mimeType} clip: ${String(error)}`);
				return null;
			});
		// Keep the encoded size with its buffer, so completed sources free the byte budget.
		this.queue.push(decoding.then((buffer) => buffer === null ? null : { buffer, bytes: bytes.length, clip }));
		if (!this.decoding) void this.schedule(this.generation);
	}

	/**
	 * Raw PCM16 mono at `rate`, played straight after what is queued: a voice
	 * front's frames, which arrive as speech is made and need no decoding.
	 */
	pushPcm(data: string, rate: number): void {
		const bytes = fromBase64(data);
		const samples = Math.floor(bytes.length / 2);
		if (samples === 0) return;
		if (this.nextAt - this.ctx.currentTime > MAX_PCM_AHEAD_S) throw new Error("The desk sent more voice audio than Hotline can play. Call again.");
		const buffer = this.ctx.createBuffer(1, samples, rate);
		const channel = buffer.getChannelData(0);
		const view = new DataView(bytes.buffer, bytes.byteOffset, samples * 2);
		for (let i = 0; i < samples; i++) channel[i] = view.getInt16(i * 2, true) / 0x8000;
		const lead = this.sources.size === 0 ? PCM_LEAD_S : 0;
		this.play(buffer, 0, this.generation, lead);
	}

	/** How much of the speech playing now has been heard, in ms: what a barge-in tells the desk was said. */
	playedMs(): number {
		if (this.spokenAt === null) return 0;
		const heard = Math.min(this.ctx.currentTime, this.nextAt) - this.spokenAt;
		return Math.max(0, Math.round(heard * 1000));
	}

	/** The last clip that began to play since playback last stopped; null when none has. */
	position(): ClipPlayed | null {
		const now = this.ctx.currentTime;
		const begun = this.scheduled.filter((clip) => clip.at <= now).at(-1);
		if (begun === undefined) return null;
		const { at, duration, ...clip } = begun;
		return { ...clip, ms: Math.round(Math.min(now - at, duration) * 1000), clipMs: Math.round(duration * 1000) };
	}

	/** Stops the sentence in flight and forgets the rest. */
	stop(): void {
		this.generation++;
		this.spokenAt = null;
		this.scheduled = [];
		this.queue = [];
		this.decoding = false;
		this.nextAt = 0;
		this.queuedBytes = 0;
		for (const timer of this.startTimers) clearTimeout(timer);
		this.startTimers.clear();
		for (const source of this.sources) {
			source.onended = null;
			try {
				source.stop();
			} catch {
				/* never started */
			}
			source.disconnect();
		}
		this.sources.clear();
	}

	private async schedule(generation: number): Promise<void> {
		this.decoding = true;
		while (generation === this.generation && this.queue.length > 0) {
			const clip = await this.queue.shift()!;
			if (generation !== this.generation) return;
			if (clip === null) continue;
			this.play(clip.buffer, clip.bytes, generation, 0, clip.clip);
		}
		if (generation !== this.generation) return;
		this.decoding = false;
		if (!this.busy) this.idle();
	}

	private play(buffer: AudioBuffer, bytes: number, generation: number, lead: number, clip?: ClipId): void {
		const source = this.ctx.createBufferSource();
		source.buffer = buffer;
		source.connect(this.out);
		const at = Math.max(this.ctx.currentTime + lead, this.nextAt);
		this.nextAt = at + buffer.duration;
		if (clip !== undefined) {
			// Only the last clip to have begun is needed of those that have.
			const now = this.ctx.currentTime;
			const begun = this.scheduled.filter((one) => one.at <= now).at(-1);
			this.scheduled = [...(begun ? [begun] : []), ...this.scheduled.filter((one) => one.at > now), { ...clip, at, duration: buffer.duration }];
		}
		this.spokenAt ??= at;
		this.sources.add(source);
		source.onended = () => {
			if (generation !== this.generation) return;
			this.sources.delete(source);
			this.queuedBytes -= bytes;
			source.disconnect();
			if (!this.busy) this.idle();
		};
		source.start(at);
		const timer = setTimeout(() => {
			this.startTimers.delete(timer);
			if (generation === this.generation) this.onStarted();
		}, Math.max(0, at - this.ctx.currentTime) * 1000);
		this.startTimers.add(timer);
	}

	private idle(): void {
		this.spokenAt = null;
		this.onIdle();
	}
}
