import { fromBase64 } from "./wav";

/**
 * The desk's voice: whole clips, one per sentence, played back to back in
 * the order they came. Decoding runs ahead of playback so the gap between
 * sentences is the clip's own silence, not a decode.
 */
export class ClipPlayer {
	private queue: Promise<AudioBuffer | null>[] = [];
	private current: AudioBufferSourceNode | null = null;
	private playing = false;
	private generation = 0;

	constructor(
		private readonly ctx: AudioContext,
		private readonly out: AudioNode,
		private readonly onIdle: () => void,
	) {}

	get busy(): boolean {
		return this.playing || this.queue.length > 0;
	}

	push(mimeType: string, data: string): void {
		const bytes = fromBase64(data);
		const decoding = this.ctx
			.decodeAudioData(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer)
			.catch((error: unknown) => {
				console.error(`Hotline could not play a ${mimeType} clip: ${String(error)}`);
				return null;
			});
		this.queue.push(decoding);
		if (!this.playing) void this.next(this.generation);
	}

	/** Stops the sentence in flight and forgets the rest. */
	stop(): void {
		this.generation++;
		this.queue = [];
		if (this.current) {
			this.current.onended = null;
			try {
				this.current.stop();
			} catch {
				/* never started */
			}
		}
		this.current = null;
		this.playing = false;
	}

	private async next(generation: number): Promise<void> {
		const decoding = this.queue.shift();
		if (!decoding) {
			this.playing = false;
			this.onIdle();
			return;
		}
		this.playing = true;
		const buffer = await decoding;
		if (generation !== this.generation) return;
		if (!buffer) return this.next(generation);
		const source = this.ctx.createBufferSource();
		source.buffer = buffer;
		source.connect(this.out);
		source.onended = () => {
			if (generation !== this.generation) return;
			this.current = null;
			void this.next(generation);
		};
		this.current = source;
		source.start();
	}
}
