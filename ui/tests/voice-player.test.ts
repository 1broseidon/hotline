import { describe, expect, test } from "bun:test";
import { ClipPlayer, MAX_QUEUED_CLIPS } from "../src/voice/player";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function rig() {
	const decodes: ((value: AudioBuffer) => void)[] = [];
	const nodes: { started: number[]; stops: number; onended: (() => void) | null; buffer: AudioBuffer | null; connect(): void; disconnect(): void; start(at: number): void; stop(): void }[] = [];
	let idle = 0;
	let started = 0;
	const ctx = {
		currentTime: 10,
		decodeAudioData: () => new Promise<AudioBuffer>((resolve) => decodes.push(resolve)),
		createBufferSource: () => {
			const node = {
				started: [] as number[], stops: 0, onended: null as (() => void) | null, buffer: null as AudioBuffer | null,
				connect() {}, disconnect() {}, start(at: number) { this.started.push(at); }, stop() { this.stops++; },
			};
			nodes.push(node);
			return node;
		},
	};
	const player = new ClipPlayer(ctx as unknown as AudioContext, {} as AudioNode, () => idle++, () => started++);
	return { player, ctx, decodes, nodes, get idle() { return idle; }, get started() { return started; } };
}

describe("progressive voice playback", () => {
	test("an unbounded decode backlog is refused instead of accumulating promises", () => {
		const r = rig();
		for (let i = 0; i < MAX_QUEUED_CLIPS; i++) r.player.push("audio/wav", "AA==");
		expect(() => r.player.push("audio/wav", "AA==")).toThrow("more voice audio");
		r.player.stop();
	});
	test("later decodes cannot overtake earlier chunks, which schedule without end-event gaps", async () => {
		const r = rig();
		r.player.push("audio/wav", "AA==");
		r.player.push("audio/wav", "AQ==");
		r.decodes[1]!({ duration: 0.2 } as AudioBuffer);
		await tick();
		expect(r.nodes).toHaveLength(0);
		r.decodes[0]!({ duration: 0.3 } as AudioBuffer);
		await tick();
		expect(r.nodes.map((node) => node.started[0])).toEqual([10, 10.3]);
		await tick();
		expect(r.started).toBe(1);
		expect(r.player.busy).toBe(true);
		r.nodes[0]!.onended!();
		expect(r.idle).toBe(0);
		r.nodes[1]!.onended!();
		expect(r.player.busy).toBe(false);
		expect(r.idle).toBe(1);
		r.player.stop();
	});

	test("cutting in stops every scheduled chunk and drops late decodes", async () => {
		const r = rig();
		r.player.push("audio/wav", "AA==");
		r.player.push("audio/wav", "AQ==");
		r.player.push("audio/wav", "Ag==");
		r.decodes[0]!({ duration: 0.2 } as AudioBuffer);
		r.decodes[1]!({ duration: 0.2 } as AudioBuffer);
		await tick();
		expect(r.nodes).toHaveLength(2);
		r.player.stop();
		r.decodes[2]!({ duration: 0.2 } as AudioBuffer);
		await tick();
		expect(r.nodes).toHaveLength(2);
		expect(r.nodes.map((node) => node.stops)).toEqual([1, 1]);
		expect(r.player.busy).toBe(false);
		expect(r.idle).toBe(0);
	});

	test("a new response can play while an old cancelled decode is unresolved", async () => {
		const r = rig();
		r.player.push("audio/wav", "AA==");
		r.player.stop();
		r.player.push("audio/wav", "AQ==");
		r.decodes[1]!({ duration: 0.2 } as AudioBuffer);
		await tick();
		expect(r.nodes).toHaveLength(1);
		r.decodes[0]!({ duration: 0.2 } as AudioBuffer);
		await tick();
		expect(r.nodes).toHaveLength(1);
		expect(r.player.busy).toBe(true);
		r.player.stop();
	});
});
