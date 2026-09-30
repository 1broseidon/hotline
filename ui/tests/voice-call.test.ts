import { describe, expect, test } from "bun:test";
Object.assign(globalThis, { requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
const { Call } = await import("../src/voice/call");
import type { CallAudio } from "../src/voice/audio";
import type { CallTransport } from "../src/voice/call";

type Handlers = { snapshot(items: unknown[]): void; event(item: unknown): void };

function rig() {
	const sent: { cmd: string; params: Record<string, unknown> }[] = [];
	let handlers: Handlers | null = null;
	let connection: ((state: "open" | "closed" | "gone") => void) | null = null;
	let callStart: (() => void) | null = null;
	let holdStart = false;
	const transport: CallTransport = {
		command: (cmd, params) => {
			sent.push({ cmd, params });
			if (cmd === "voice.call_start" && holdStart) return new Promise((resolve) => (callStart = () => resolve(null)));
			return Promise.resolve(null);
		},
		subscribe: (_target, h) => {
			handlers = h;
			return () => (handlers = null);
		},
		onConnection: (watcher) => {
			connection = watcher;
			return () => (connection = null);
		},
	};
	const played: string[] = [];
	let queue = 0;
	let idle: () => void = () => {};
	const mic = { open: true, closed: 0 };
	const audio: CallAudio = {
		open: async () => {},
		closeMic: () => {
			mic.open = false;
			mic.closed++;
		},
		reopenMic: async () => {
			mic.open = true;
		},
		play: (_mime, data) => {
			played.push(data);
			queue++;
		},
		stopPlayback: () => {
			queue = 0;
		},
		get playing() {
			return queue > 0;
		},
		outputLevel: () => 0,
		chime: () => 0,
		close: () => {
			mic.open = false;
		},
	};
	let now = 1_000;
	const call = new Call(transport, () => "Mack", audio, () => now);
	idle = () => call.settle();
	return {
		call,
		sent,
		played,
		mic,
		desk: (event: unknown) => handlers?.event(event),
		snapshot: (items: unknown[]) => handlers?.snapshot(items),
		connection: (state: "open" | "closed" | "gone") => connection?.(state),
		finishClip: () => {
			queue = Math.max(0, queue - 1);
			if (queue === 0) idle();
		},
		holdStart: () => (holdStart = true),
		releaseStart: () => callStart?.(),
		speak: (level: number, fromMs: number, toMs: number) => {
			for (now = fromMs; now <= toMs; now += 43) call.hear(new Float32Array(2048).fill(level), 48_000);
		},
	};
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("a call with the desk", () => {
	test("a spoken turn is sent once, and the call waits on the desk", async () => {
		const r = rig();
		await r.call.start();
		expect(r.call.current.phase).toBe("listening");
		r.speak(0.3, 1_000, 1_600);
		expect(r.call.current.phase).toBe("hearing");
		r.speak(0, 1_650, 2_500);
		expect(r.sent.filter((one) => one.cmd === "voice.utterance")).toHaveLength(1);
		expect(r.call.current.phase).toBe("thinking");
	});

	test("the desk thinking while a clip plays does not leave the mic shut", async () => {
		const r = rig();
		await r.call.start();
		r.desk({ type: "said", id: "s1", text: "On it." });
		r.desk({ type: "clip", id: "s1", index: 0, final: true, mimeType: "audio/wav", data: "one" });
		expect(r.call.current.phase).toBe("speaking");
		r.desk({ type: "state", state: "thinking" });
		expect(r.call.current.phase).toBe("speaking");
		r.finishClip();
		expect(r.call.current.phase).toBe("thinking");
		r.desk({ type: "state", state: "listening" });
		expect(r.call.current.phase).toBe("listening");
	});

	test("the desk saying listening after its last clip opens the mic once playback ends", async () => {
		const r = rig();
		await r.call.start();
		r.desk({ type: "state", state: "speaking" });
		r.desk({ type: "clip", id: "s1", index: 0, final: true, mimeType: "audio/wav", data: "one" });
		r.desk({ type: "state", state: "listening" });
		expect(r.call.current.phase).toBe("speaking");
		r.finishClip();
		expect(r.call.current.phase).toBe("listening");
	});

	test("clips from a turn that was cut in on are not played", async () => {
		const r = rig();
		await r.call.start();
		r.desk({ type: "said", id: "s1", text: "A long answer." });
		r.desk({ type: "clip", id: "s1", index: 0, final: false, mimeType: "audio/wav", data: "one" });
		r.call.interrupt();
		expect(r.sent.some((one) => one.cmd === "voice.interrupt")).toBe(true);
		r.desk({ type: "clip", id: "s1", index: 1, final: true, mimeType: "audio/wav", data: "late" });
		expect(r.played).toEqual(["one"]);
		expect(r.call.current.phase).toBe("listening");
		r.desk({ type: "said", id: "s2", text: "New answer." });
		r.desk({ type: "clip", id: "s2", index: 0, final: true, mimeType: "audio/wav", data: "new" });
		expect(r.played).toEqual(["one", "new"]);
	});

	test("a snapshot after a reconnect never plays audio again", async () => {
		const r = rig();
		await r.call.start();
		r.snapshot([
			{ type: "state", state: "listening" },
			{ type: "clip", id: "s1", index: 0, final: true, mimeType: "audio/wav", data: "replayed" },
		]);
		expect(r.played).toEqual([]);
	});

	test("a desk out of reach ends the call: the desk has ended it too", async () => {
		const r = rig();
		await r.call.start();
		r.connection("open");
		expect(r.call.current.phase).toBe("listening");
		r.connection("closed");
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.trouble).toBe("Lost the connection to the desk.");
	});

	test("a desk that no longer pairs ends the call at once", async () => {
		const r = rig();
		await r.call.start();
		r.connection("gone");
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.trouble).toBe("This desk is no longer paired with this computer.");
	});

	test("hold lets the microphone go and resume takes it back", async () => {
		const r = rig();
		await r.call.start();
		await r.call.hold(true);
		expect(r.call.current.phase).toBe("held");
		expect(r.mic.open).toBe(false);
		r.desk({ type: "clip", id: "s1", index: 0, final: true, mimeType: "audio/wav", data: "held" });
		expect(r.played).toEqual([]);
		await r.call.hold(false);
		expect(r.mic.open).toBe(true);
		expect(r.call.current.phase).toBe("listening");
	});

	test("the clock stops on hold", async () => {
		const r = rig();
		await r.call.start();
		await r.call.hold(true);
		expect(r.call.current.clock.since).toBeNull();
		await r.call.hold(false);
		expect(r.call.current.clock.since).not.toBeNull();
	});

	test("a goodbye finishes its sentence, and cutting in during it ends the call at once", async () => {
		const r = rig();
		await r.call.start();
		r.desk({ type: "clip", id: "bye", index: 0, final: true, mimeType: "audio/wav", data: "goodbye" });
		r.desk({ type: "state", state: "ended", reason: "goodbye" });
		expect(r.call.current.phase).toBe("speaking");
		r.call.interrupt();
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.ended).toBe("goodbye");
	});

	test("hanging up before the desk has made the call still tells it", async () => {
		const r = rig();
		r.holdStart();
		const starting = r.call.start();
		await tick();
		r.call.hangUp();
		r.releaseStart();
		await starting;
		expect(r.sent.map((one) => one.cmd)).toEqual(["voice.call_start", "voice.call_end"]);
		expect(r.call.current.phase).toBe("ended");
	});

	test("an utterance the desk refuses says so and listens again", async () => {
		const r = rig();
		await r.call.start();
		(r.call as unknown as { transport: CallTransport }).transport.command = async (cmd) => {
			if (cmd === "voice.utterance") throw new Error("Wait for the answer.");
			return null;
		};
		r.speak(0.3, 1_000, 1_600);
		r.speak(0, 1_650, 2_500);
		await tick();
		expect(r.call.current.trouble).toBe("Wait for the answer.");
		expect(r.call.current.phase).toBe("listening");
	});
});
