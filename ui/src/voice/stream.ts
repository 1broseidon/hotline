import { WAV_RATE, toBase64 } from "./wav";

/** 200ms chunks amortize wire acknowledgements without waiting for the turn to end. */
const CHUNK_SAMPLES = WAV_RATE / 5;
export const MAX_PCM_SAMPLES = WAV_RATE * 20;
/** Four seconds of unsent audio. A stalled connection must not grow a promise queue forever. */
export const MAX_PENDING_BYTES = WAV_RATE * 2 * 4;

export type AudioChunk = { index: number; data: string; final: boolean };

/** A continuous box filter; its fractional position carries across microphone blocks. */
export class PcmResampler {
	private inputAt = 0;
	private outputAt = 0;
	private sum = 0;
	private count = 0;
	constructor(private readonly rate: number) {
		if (!Number.isFinite(rate) || rate < WAV_RATE) throw new Error("The microphone runs slower than the desk listens.");
	}
	push(block: Float32Array): Int16Array {
		const samples: number[] = [];
		const ratio = this.rate / WAV_RATE;
		for (const value of block) {
			this.sum += Number.isFinite(value) ? value : 0;
			this.count++;
			this.inputAt++;
			if (this.inputAt < Math.ceil((this.outputAt + 1) * ratio)) continue;
			const sample = Math.max(-1, Math.min(1, this.sum / this.count));
			samples.push(Math.trunc(sample < 0 ? sample * 0x8000 : sample * 0x7fff));
			this.outputAt++;
			this.sum = 0;
			this.count = 0;
		}
		return Int16Array.from(samples);
	}
}

/** One utterance, with one acknowledgement in flight and a bounded ordered send queue. */
export class PcmTurn {
	private readonly resampler: PcmResampler;
	private samples = new Int16Array(CHUNK_SAMPLES);
	private used = 0;
	private total = 0;
	private index = 0;
	private queue: { bytes: Uint8Array; final: boolean }[] = [];
	private pendingBytes = 0;
	private running = false;
	private cancelled = false;
	private finished = false;
	private resolve: (() => void) | undefined;
	private reject: ((error: unknown) => void) | undefined;

	constructor(
		rate: number,
		private readonly send: (chunk: AudioChunk) => Promise<unknown>,
		private readonly onError: (error: unknown) => void,
	) {
		this.resampler = new PcmResampler(rate);
	}

	/** Returns true when the 20-second wire limit has been reached. */
	push(block: Float32Array): boolean {
		if (this.cancelled || this.finished) return false;
		for (const sample of this.resampler.push(block)) {
			if (this.total === MAX_PCM_SAMPLES) return true;
			this.samples[this.used++] = sample;
			this.total++;
			if (this.used === CHUNK_SAMPLES) this.flush(false);
			if (this.cancelled) return false;
		}
		return this.total === MAX_PCM_SAMPLES;
	}

	finish(): Promise<void> {
		if (this.cancelled) return Promise.reject(new Error("The audio turn was cancelled."));
		if (this.finished) return Promise.reject(new Error("The audio turn is already committed."));
		this.finished = true;
		return new Promise((resolve, reject) => {
			this.resolve = resolve;
			this.reject = reject;
			// An empty last chunk is a valid commit when the preceding chunk filled exactly.
			this.flush(true);
		});
	}

	cancel(): void {
		this.cancelled = true;
		this.queue = [];
		this.pendingBytes = 0;
		this.used = 0;
		this.reject?.(new Error("The audio turn was cancelled."));
		this.resolve = this.reject = undefined;
	}

	private flush(final: boolean): void {
		const bytes = new Uint8Array(this.used * 2);
		const view = new DataView(bytes.buffer);
		for (let i = 0; i < this.used; i++) view.setInt16(i * 2, this.samples[i]!, true);
		this.used = 0;
		if (this.pendingBytes + bytes.length > MAX_PENDING_BYTES) {
			this.fail(new Error("The connection is too slow to carry live audio. Call again when it is back."));
			return;
		}
		this.queue.push({ bytes, final });
		this.pendingBytes += bytes.length;
		if (!this.running) void this.drain();
	}

	private async drain(): Promise<void> {
		this.running = true;
		try {
			while (!this.cancelled && this.queue.length > 0) {
				const first = this.queue.shift()!;
				const parts = [first.bytes];
				let length = first.bytes.length;
				let final = first.final;
				// The first packet leaves promptly. While an ack is in flight, adjacent
				// packets coalesce up to the wire limit so a slower RTT can still keep up.
				while (!final && this.queue[0] !== undefined && length + this.queue[0].bytes.length <= 32_768) {
					const next = this.queue.shift()!;
					parts.push(next.bytes);
					length += next.bytes.length;
					final = next.final;
				}
				const bytes = parts.length === 1 ? first.bytes : new Uint8Array(length);
				if (parts.length > 1) {
					let at = 0;
					for (const part of parts) { bytes.set(part, at); at += part.length; }
				}
				await this.send({ index: this.index++, data: toBase64(bytes), final });
				if (this.cancelled) return;
				this.pendingBytes -= bytes.length;
				if (final) {
					this.resolve?.();
					this.resolve = this.reject = undefined;
				}
			}
		} catch (error) {
			if (!this.cancelled) this.fail(error);
		} finally {
			this.running = false;
		}
	}

	private fail(error: unknown): void {
		this.reject?.(error);
		this.resolve = this.reject = undefined;
		this.cancel();
		this.onError(error);
	}
}
