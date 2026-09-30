import { describe, expect, test } from "bun:test";
import { downsample, encodeWav, fromBase64, rms, toBase64 } from "../src/voice/wav";

describe("a turn as the desk takes it", () => {
	test("48 kHz comes down to 16 kHz by averaging", () => {
		const input = new Float32Array([0.3, 0.3, 0.3, -0.6, -0.6, -0.6]);
		const out = downsample(input, 48_000, 16_000);
		expect(Array.from(out).map((v) => Math.round(v * 10) / 10)).toEqual([0.3, -0.6]);
	});

	test("a microphone slower than the desk listens is refused", () => {
		expect(() => downsample(new Float32Array(4), 8_000, 16_000)).toThrow();
	});

	test("the WAV header says 16 kHz mono PCM16 and the data follows it", () => {
		const wav = encodeWav(new Float32Array([0, 1, -1]), 16_000);
		const view = new DataView(wav.buffer);
		const text = (at: number) => String.fromCharCode(...wav.subarray(at, at + 4));
		expect(text(0)).toBe("RIFF");
		expect(text(8)).toBe("WAVE");
		expect(view.getUint16(22, true)).toBe(1);
		expect(view.getUint32(24, true)).toBe(16_000);
		expect(view.getUint16(34, true)).toBe(16);
		expect(view.getUint32(40, true)).toBe(6);
		expect(view.getInt16(46, true)).toBe(0x7fff);
		expect(view.getInt16(48, true)).toBe(-0x8000);
		expect(wav.length).toBe(50);
	});

	test("base64 goes there and back, past the chunk size", () => {
		const bytes = new Uint8Array(100_000).map((_, i) => i % 251);
		expect(fromBase64(toBase64(bytes))).toEqual(bytes);
	});

	test("silence has no level, a full-scale square wave has one", () => {
		expect(rms(new Float32Array(8))).toBe(0);
		expect(rms(new Float32Array([1, -1, 1, -1]))).toBe(1);
	});
});
