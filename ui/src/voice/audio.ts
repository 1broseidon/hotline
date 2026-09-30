import { ClipPlayer } from "./player";
import { rms } from "./wav";

/**
 * Everything a call needs from the webview's audio, behind one seam so the
 * call itself can be driven by a fake in tests: the microphone as blocks of
 * samples, playback of whole clips, the output level, and the earcons.
 */
export interface CallAudio {
	/** Asks for the microphone and starts reading it; rejects when it cannot. */
	open(onBlock: (block: Float32Array, rate: number) => void): Promise<void>;
	/** Lets the microphone go, so the system's mic light goes off (hold). */
	closeMic(): void;
	/** Takes the microphone again after `closeMic`. */
	reopenMic(): Promise<void>;
	/** Queues one playable clip; `onIdle` fires once the queue has drained. */
	play(mimeType: string, data: string): void;
	stopPlayback(): void;
	readonly playing: boolean;
	/** The loudness of what is playing now, as RMS. */
	outputLevel(): number;
	/** A soft tone; answers its length in seconds. */
	chime(kind: "connect" | "think" | "end"): number;
	close(): void;
}

/** A block of the mic is this many frames: ~43ms at 48 kHz. */
const BLOCK = 2048;

export function webAudio(events: { onIdle(): void; onLost(): void }): CallAudio {
	let ctx: AudioContext | null = null;
	let stream: MediaStream | null = null;
	let source: MediaStreamAudioSourceNode | null = null;
	let processor: ScriptProcessorNode | null = null;
	let analyser: AnalyserNode | null = null;
	let player: ClipPlayer | null = null;
	let reader: ((block: Float32Array, rate: number) => void) | null = null;
	let closing = false;

	const takeMic = async () => {
		const context = ctx;
		if (!context) throw new Error("closed");
		stream = await navigator.mediaDevices.getUserMedia({
			audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1 },
		});
		// A mic unplugged or taken by the system mid-call.
		for (const track of stream.getAudioTracks()) {
			track.onended = () => {
				if (!closing && stream?.getAudioTracks().includes(track)) events.onLost();
			};
		}
		source = context.createMediaStreamSource(stream);
		source.connect(processor!);
	};

	const dropMic = () => {
		source?.disconnect();
		source = null;
		for (const track of stream?.getTracks() ?? []) {
			track.onended = null;
			track.stop();
		}
		stream = null;
	};

	return {
		async open(onBlock) {
			reader = onBlock;
			const context = new AudioContext();
			ctx = context;
			// A suspended context (the system took audio away) is asked back.
			context.onstatechange = () => {
				if (!closing && context.state === "suspended") void context.resume().catch(() => {});
			};
			await context.resume();
			const out = context.createGain();
			analyser = context.createAnalyser();
			analyser.fftSize = 1024;
			out.connect(analyser);
			analyser.connect(context.destination);
			player = new ClipPlayer(context, out, events.onIdle);
			processor = context.createScriptProcessor(BLOCK, 1, 1);
			processor.onaudioprocess = (event) => reader?.(event.inputBuffer.getChannelData(0), context.sampleRate);
			// A script processor only runs while it leads somewhere; this gain is silent.
			const sink = context.createGain();
			sink.gain.value = 0;
			processor.connect(sink);
			sink.connect(context.destination);
			await takeMic();
		},
		closeMic: dropMic,
		reopenMic: takeMic,
		play(mimeType, data) {
			player?.push(mimeType, data);
		},
		stopPlayback() {
			player?.stop();
		},
		get playing() {
			return player?.busy ?? false;
		},
		outputLevel() {
			if (!analyser) return 0;
			const buffer = new Float32Array(analyser.fftSize);
			analyser.getFloatTimeDomainData(buffer);
			return rms(buffer);
		},
		chime(kind) {
			const context = ctx;
			if (!context || context.state !== "running") return 0;
			const notes = { connect: [587.33, 880], think: [1174.66], end: [880, 587.33] }[kind];
			const peak = kind === "think" ? 0.035 : 0.06;
			const length = kind === "think" ? 0.07 : 0.11;
			const step = 0.09;
			const t0 = context.currentTime + 0.02;
			notes.forEach((frequency, i) => {
				const at = t0 + i * step;
				const osc = context.createOscillator();
				const gain = context.createGain();
				osc.type = "sine";
				osc.frequency.value = frequency;
				gain.gain.setValueAtTime(0, at);
				gain.gain.linearRampToValueAtTime(peak, at + 0.008);
				gain.gain.exponentialRampToValueAtTime(0.0001, at + length);
				osc.connect(gain).connect(context.destination);
				osc.start(at);
				osc.stop(at + length + 0.02);
			});
			return 0.02 + (notes.length - 1) * step + length;
		},
		close() {
			closing = true;
			reader = null;
			player?.stop();
			if (processor) processor.onaudioprocess = null;
			processor?.disconnect();
			dropMic();
			void ctx?.close().catch(() => {});
			ctx = null;
		},
	};
}
