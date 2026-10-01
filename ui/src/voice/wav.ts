/**
 * One utterance, as the desk takes it: 16 kHz mono PCM16 in a WAV. The
 * webview records at whatever rate its audio context runs (44.1 or 48 kHz),
 * so the samples are averaged down first; speech lives well under 8 kHz and
 * a box filter is enough for a transcriber.
 */
export const WAV_RATE = 16_000;

/** Averages `input` at `from` Hz down to `to` Hz. */
export function downsample(input: Float32Array, from: number, to: number = WAV_RATE): Float32Array {
	if (from === to) return input.slice();
	if (from < to) throw new Error("The microphone runs slower than the desk listens.");
	const ratio = from / to;
	const length = Math.floor(input.length / ratio);
	const out = new Float32Array(length);
	for (let i = 0; i < length; i++) {
		const start = Math.floor(i * ratio);
		const end = Math.min(input.length, Math.floor((i + 1) * ratio));
		let sum = 0;
		for (let j = start; j < end; j++) sum += input[j] ?? 0;
		out[i] = end > start ? sum / (end - start) : 0;
	}
	return out;
}

/** A RIFF/WAVE file of `samples` (-1..1) at `rate`, mono, 16-bit. */
export function encodeWav(samples: Float32Array, rate: number = WAV_RATE): Uint8Array {
	const bytes = new Uint8Array(44 + samples.length * 2);
	const view = new DataView(bytes.buffer);
	const ascii = (at: number, text: string) => {
		for (let i = 0; i < text.length; i++) view.setUint8(at + i, text.charCodeAt(i));
	};
	ascii(0, "RIFF");
	view.setUint32(4, 36 + samples.length * 2, true);
	ascii(8, "WAVE");
	ascii(12, "fmt ");
	view.setUint32(16, 16, true);
	view.setUint16(20, 1, true); // PCM
	view.setUint16(22, 1, true); // mono
	view.setUint32(24, rate, true);
	view.setUint32(28, rate * 2, true);
	view.setUint16(32, 2, true);
	view.setUint16(34, 16, true);
	ascii(36, "data");
	view.setUint32(40, samples.length * 2, true);
	for (let i = 0; i < samples.length; i++) {
		const s = Math.max(-1, Math.min(1, samples[i] ?? 0));
		view.setInt16(44 + i * 2, s < 0 ? s * 0x8000 : s * 0x7fff, true);
	}
	return bytes;
}

/** Standard padded base64, the way the wire carries audio. */
export function toBase64(bytes: Uint8Array): string {
	let binary = "";
	const step = 0x8000;
	for (let i = 0; i < bytes.length; i += step) {
		binary += String.fromCharCode(...bytes.subarray(i, i + step));
	}
	return btoa(binary);
}

export function fromBase64(data: string): Uint8Array {
	const binary = atob(data);
	const bytes = new Uint8Array(binary.length);
	for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
	return bytes;
}

/** Root mean square of a block of samples: the level the turn detector reads. */
export function rms(samples: Float32Array): number {
	let sum = 0;
	for (let i = 0; i < samples.length; i++) {
		const s = samples[i] ?? 0;
		sum += s * s;
	}
	return samples.length ? Math.sqrt(sum / samples.length) : 0;
}
