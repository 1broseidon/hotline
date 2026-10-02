import { describe, expect, test } from "bun:test";
import { type AudioChunk, MAX_PCM_SAMPLES, PcmResampler, PcmTurn } from "../src/voice/stream";
import { encodeWav, fromBase64 } from "../src/voice/wav";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("streamed microphone audio", () => {
	test("44.1kHz resampling carries its position across irregular blocks", () => {
		const input = Float32Array.from({ length: 44_100 }, (_, i) => Math.sin(i / 13) * 0.5);
		const whole = new PcmResampler(44_100).push(input);
		const split = new PcmResampler(44_100);
		const samples: number[] = [];
		for (let at = 0; at < input.length; at += 127) samples.push(...split.push(input.subarray(at, at + 127)));
		expect(samples).toEqual(Array.from(whole));
		expect(samples).toHaveLength(16_000);
	});

	test("one ack is in flight and queued PCM coalesces in order with its final commit", async () => {
		const sent: AudioChunk[] = [];
		const acks: (() => void)[] = [];
		const errors: unknown[] = [];
		const turn = new PcmTurn(16_000, (chunk) => {
			sent.push(chunk);
			return new Promise<void>((resolve) => acks.push(resolve));
		}, (error) => errors.push(error));
		const input = Float32Array.from({ length: 12_800 }, (_, i) => i % 2 === 0 ? 0.5 : -0.5);
		turn.push(input);
		const committed = turn.finish();
		expect(sent).toHaveLength(1);
		for (let index = 0; index < 2; index++) {
			expect(sent.at(-1)!.index).toBe(index);
			acks.shift()!();
			await tick();
		}
		await committed;
		expect(sent.map((chunk) => chunk.final)).toEqual([false, true]);
		expect(sent.map((chunk) => fromBase64(chunk.data).length)).toEqual([6_400, 19_200]);
		const actual = Uint8Array.from(sent.flatMap((chunk) => Array.from(fromBase64(chunk.data))));
		expect(actual).toEqual(encodeWav(input).subarray(44));
		expect(errors).toEqual([]);
		expect(sent.every((chunk) => fromBase64(chunk.data).length <= 32_768)).toBe(true);
	});

	test("coalescing cannot exceed 32KiB and a commit with no remaining data is valid", async () => {
		const sent: AudioChunk[] = [];
		const acks: (() => void)[] = [];
		const turn = new PcmTurn(16_000, (chunk) => {
			sent.push(chunk);
			return new Promise<void>((resolve) => acks.push(resolve));
		}, () => {});
		turn.push(new Float32Array(3_200));
		acks.shift()!();
		await tick();
		const emptyCommit = turn.finish();
		expect(sent[1]).toEqual({ index: 1, data: "", final: true });
		acks.shift()!();
		await emptyCommit;
		const second = new PcmTurn(16_000, (chunk) => {
			sent.push(chunk);
			return new Promise<void>((resolve) => acks.push(resolve));
		}, () => {});
		second.push(new Float32Array(3_200 * 8));
		const committed = second.finish();
		while (acks.length > 0) { acks.shift()!(); await tick(); }
		await committed;
		expect(sent.slice(2).map((chunk) => fromBase64(chunk.data).length)).toEqual([6_400, 32_000, 12_800]);
		expect(sent.slice(2).map((chunk) => chunk.index)).toEqual([0, 1, 2]);
		expect(sent.at(-1)!.final).toBe(true);
	});

	test("cancel drops unsent audio and cannot send a delayed final", async () => {
		const sent: AudioChunk[] = [];
		let ack: (() => void) | undefined;
		const turn = new PcmTurn(16_000, (chunk) => {
			sent.push(chunk);
			return new Promise<void>((resolve) => (ack = resolve));
		}, () => {});
		turn.push(new Float32Array(9_600));
		const committed = turn.finish();
		turn.cancel();
		ack!();
		await expect(committed).rejects.toThrow("cancelled");
		await tick();
		expect(sent).toHaveLength(1);
		expect(sent[0]!.final).toBe(false);
	});

	test("a stalled ack has a bounded queue and reports failure once", async () => {
		const sent: AudioChunk[] = [];
		const errors: unknown[] = [];
		let ack: (() => void) | undefined;
		const turn = new PcmTurn(16_000, (chunk) => {
			sent.push(chunk);
			return new Promise<void>((resolve) => (ack = resolve));
		}, (error) => errors.push(error));
		for (let i = 0; i < 100; i++) turn.push(new Float32Array(3_200));
		expect(sent).toHaveLength(1);
		expect(errors).toHaveLength(1);
		ack!();
		await tick();
		expect(sent).toHaveLength(1);
	});

	test("a turn commits at at most twenty seconds of PCM including pre-roll", async () => {
		const sent: AudioChunk[] = [];
		const turn = new PcmTurn(16_000, async (chunk) => { sent.push(chunk); }, () => {});
		for (let i = 0; i < 101; i++) {
			const full = turn.push(new Float32Array(3_200));
			await tick();
			if (i === 99) expect(full).toBe(true);
		}
		await turn.finish();
		expect(sent.reduce((count, chunk) => count + fromBase64(chunk.data).length / 2, 0)).toBe(MAX_PCM_SAMPLES);
		expect(sent.at(-1)!.final).toBe(true);
	});

	test("wire failures discard the queue and reject the commit", async () => {
		const sent: AudioChunk[] = [];
		const errors: unknown[] = [];
		const turn = new PcmTurn(16_000, async (chunk) => {
			sent.push(chunk);
			throw new Error("Disconnected");
		}, (error) => errors.push(error));
		turn.push(new Float32Array(9_600));
		await expect(turn.finish()).rejects.toThrow("Disconnected");
		expect(sent).toHaveLength(1);
		expect(errors).toHaveLength(1);
	});
});
