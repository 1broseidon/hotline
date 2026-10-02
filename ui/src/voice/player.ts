import { fromBase64 } from "./wav";

export const MAX_QUEUED_CLIPS = 128;
const MAX_QUEUED_BYTES = 8 * 1024 * 1024;

/** Decode ahead and schedule consecutive clips on the audio clock, including progressive WAV chunks. */
export class ClipPlayer {
	private readonly sources = new Set<AudioBufferSourceNode>();
	private queue: Promise<{ buffer: AudioBuffer; bytes: number } | null>[] = [];
	private decoding = false;
	private nextAt = 0;
	private generation = 0;
	private queuedBytes = 0;
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

	push(mimeType: string, data: string): void {
		if (this.queue.length + this.sources.size + Number(this.decoding) >= MAX_QUEUED_CLIPS || data.length > MAX_QUEUED_BYTES * 4 / 3 + 4) {
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
		this.queue.push(decoding.then((buffer) => buffer === null ? null : { buffer, bytes: bytes.length }));
		if (!this.decoding) void this.schedule(this.generation);
	}

	/** Stops the sentence in flight and forgets the rest. */
	stop(): void {
		this.generation++;
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
			const { buffer, bytes } = clip;
			const source = this.ctx.createBufferSource();
			source.buffer = buffer;
			source.connect(this.out);
			const at = Math.max(this.ctx.currentTime, this.nextAt);
			this.nextAt = at + buffer.duration;
			this.sources.add(source);
			source.onended = () => {
				if (generation !== this.generation) return;
				this.sources.delete(source);
				this.queuedBytes -= bytes;
				source.disconnect();
				if (!this.busy) this.onIdle();
			};
			source.start(at);
			const timer = setTimeout(() => {
				this.startTimers.delete(timer);
				if (generation === this.generation) this.onStarted();
			}, Math.max(0, at - this.ctx.currentTime) * 1000);
			this.startTimers.add(timer);
		}
		if (generation !== this.generation) return;
		this.decoding = false;
		if (!this.busy) this.onIdle();
	}
}
