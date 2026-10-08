import { describe, expect, test } from "bun:test";
Object.assign(globalThis, { requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
const { Call, supportsDirectCalls } = await import("../src/voice/call");
import type { CallAudio } from "../src/voice/audio";
import type { CallOptions, CallTransport } from "../src/voice/call";
import type { DeviceTranscription, TranscriptionCapability, TranscriptionEvent } from "../src/voice/transcription";

type Handlers = { snapshot(items: unknown[]): void; event(item: unknown): void };

function rig(options: CallOptions = {}, response: unknown = null, capabilities: string[] = ["voice", "voiceDirectCalls"], transcription?: DeviceTranscription) {
	const sent: { cmd: string; params: Record<string, unknown> }[] = [];
	let handlers: Handlers | null = null;
	let connection: ((state: "open" | "closed" | "gone") => void) | null = null;
	let callStart: (() => void) | null = null;
	let holdStart = false;
	let holdAudio = false;
	const audioAcks: (() => void)[] = [];
	let resumeMic: (() => void) | null = null;
	let holdResume = false;
	let status: unknown = { available: true, capabilities };
	const transport: CallTransport = {
		command: (cmd, params) => {
			sent.push({ cmd, params });
			if (cmd === "voice.status") return Promise.resolve(status);
			if (cmd === "voice.audio" && holdAudio) return new Promise((resolve) => audioAcks.push(() => resolve(null)));
			if (cmd === "voice.call_start" && holdStart) return new Promise((resolve) => (callStart = () => resolve(null)));
			if (cmd === "voice.call_start") return Promise.resolve(response);
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
	const mic = { open: true, closed: 0, opened: 0 };
	const audio: CallAudio = {
		open: async () => {},
		closeMic: () => {
			mic.open = false;
			mic.closed++;
		},
		openMic: async () => {
			if (holdResume) await new Promise<void>((resolve) => (resumeMic = resolve));
			mic.open = true;
			mic.opened++;
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
	const call = new Call(transport, () => "Mack", audio, () => now, options, transcription);
	idle = () => call.settle();
	return {
		call,
		status: (value: unknown) => (status = value),
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
		holdAudio: () => (holdAudio = true),
		ackAudio: () => audioAcks.shift()?.(),
		holdResume: () => (holdResume = true),
		releaseResume: () => resumeMic?.(),
		at: (ms: number) => (now = ms),
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
		r.speak(0, 1_650, 3_000);
		expect(r.sent.filter((one) => one.cmd === "voice.utterance")).toHaveLength(1);
		expect(r.call.current.phase).toBe("thinking");
	});

	test("an unconfirmed room hum is learned without sending an utterance", async () => {
		const r = rig();
		await r.call.start();
		r.speak(0.015, 1_000, 6_000);
		expect(r.sent.filter((one) => one.cmd === "voice.utterance")).toHaveLength(0);
		expect(r.call.current.phase).toBe("listening");
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

	test("cutting in on unfinished work waits for the desk to say it can listen", async () => {
		const r = rig();
		await r.call.start();
		r.desk({ type: "state", state: "thinking" });
		r.desk({ type: "clip", id: "ack", index: 0, final: true, mimeType: "audio/wav", data: "one moment" });
		r.call.interrupt();
		expect(r.call.current.phase).toBe("thinking");
		r.desk({ type: "state", state: "listening" });
		expect(r.call.current.phase).toBe("listening");
	});

	test("a failure is said before the line goes", async () => {
		const r = rig();
		await r.call.start();
		r.desk({ type: "clip", id: "sorry", index: 0, final: true, mimeType: "audio/wav", data: "sorry" });
		r.desk({ type: "state", state: "ended", reason: "error" });
		expect(r.call.current.phase).toBe("speaking");
		r.finishClip();
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.ended).toBe("error");
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
		r.speak(0, 1_650, 3_000);
		await tick();
		expect(r.call.current.trouble).toBe("Wait for the answer.");
		expect(r.call.current.phase).toBe("listening");
	});

	test("desk calls omit a target and gracefully keep whole WAV input on an older desk", async () => {
		const r = rig();
		await r.call.start();
		expect(r.sent[0]!.params).toEqual({ callId: r.call.id, streamAudio: true });
		r.speak(0.3, 1_000, 1_600);
		r.speak(0, 1_650, 3_000);
		expect(r.sent.filter((one) => one.cmd === "voice.audio")).toHaveLength(0);
		expect(r.sent.filter((one) => one.cmd === "voice.utterance")).toHaveLength(1);
		r.call.hangUp();
	});

	test("a direct call refuses an old desk before creating a desk call", async () => {
		const r = rig({ target: { personaId: "mack", name: "Mack" } }, null, ["voice"]);
		await r.call.start();
		expect(r.sent.map((one) => one.cmd)).toEqual(["voice.status"]);
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.trouble).toContain("needs an update");
	});

	test("a ready direct call starts even when the desk dispatcher is unavailable", async () => {
		const r = rig({ target: { personaId: "mack", name: "Mack" } }, { personaId: "mack", input: ["audio/wav"] });
		r.status({ available: false, directAvailable: true, capabilities: ["voiceDirectCalls"], unavailable: "Connect the dispatcher." });
		await r.call.start();
		expect(r.sent.map((one) => one.cmd)).toEqual(["voice.status", "voice.call_start"]);
		expect(r.call.current.phase).toBe("listening");
		r.call.hangUp();
	});

	test("direct readiness preserves older desks and refuses explicit speech or budget failures", async () => {
		expect(supportsDirectCalls({ available: true, capabilities: ["voiceDirectCalls"] })).toBe(true);
		expect(supportsDirectCalls({ available: false, capabilities: ["voiceDirectCalls"] })).toBe(false);
		expect(supportsDirectCalls({ available: true, directAvailable: false, capabilities: ["voiceDirectCalls"] })).toBe(false);
		expect(supportsDirectCalls({ available: true, directAvailable: true, capabilities: [] })).toBe(false);
		const r = rig({ target: { personaId: "mack", name: "Mack" } }, { personaId: "mack" });
		r.status({ available: false, directAvailable: false, capabilities: ["voiceDirectCalls"], unavailable: "Today's voice budget is spent." });
		await r.call.start();
		expect(r.sent.map((one) => one.cmd)).toEqual(["voice.status"]);
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.trouble).toContain("budget is spent");
	});

	test("an ignored direct target is ended instead of silently becoming a desk call", async () => {
		const r = rig({ target: { personaId: "mack", name: "Mack" } }, { input: ["audio/wav"] });
		await r.call.start();
		expect(r.sent.map((one) => one.cmd)).toEqual(["voice.status", "voice.call_start", "voice.call_end"]);
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.trouble).toContain("chosen teammate");
	});

	test("direct calls preserve their target and desk when redialed", async () => {
		const target = { personaId: "mack", name: "Mack", avatarHash: "face" };
		const r = rig({ deskId: "desk-a", target }, { input: ["audio/wav"], personaId: "mack" });
		await r.call.start();
		expect(r.sent.find((one) => one.cmd === "voice.call_start")!.params.personaId).toBe("mack");
		expect(r.call.redial().target).toEqual(target);
		expect(r.call.redial().deskId).toBe("desk-a");
		r.call.hangUp();
	});

	test("negotiated PCM starts during speech and commits once after a tolerated pause", async () => {
		const r = rig({}, { input: ["audio/pcm", "audio/wav"] });
		await r.call.start();
		r.speak(0.3, 1_000, 1_600);
		await tick();
		expect(r.sent.some((one) => one.cmd === "voice.audio")).toBe(true);
		expect(r.sent.filter((one) => one.cmd === "voice.audio").every((one) => one.params.final === false)).toBe(true);
		r.speak(0, 1_650, 2_400); // A 900ms pause still belongs to this turn.
		expect(r.call.current.phase).toBe("hearing");
		r.speak(0.3, 2_450, 2_800);
		r.speak(0, 2_850, 4_200);
		await tick();
		const chunks = r.sent.filter((one) => one.cmd === "voice.audio");
		expect(chunks.map((one) => one.params.index)).toEqual(chunks.map((_, index) => index));
		expect(new Set(chunks.map((one) => one.params.seq))).toEqual(new Set([1]));
		expect(chunks.filter((one) => one.params.final === true)).toHaveLength(1);
		expect(chunks.at(-1)!.params.final).toBe(true);
		expect(r.sent.some((one) => one.cmd === "voice.utterance")).toBe(false);
		expect(r.call.current.phase).toBe("thinking");
		r.call.hangUp();
	});

	test("hold discards queued PCM and the partial turn without a final commit", async () => {
		const r = rig({}, { input: ["audio/pcm"] });
		await r.call.start();
		r.holdAudio();
		r.speak(0.3, 1_000, 2_000);
		expect(r.sent.filter((one) => one.cmd === "voice.audio")).toHaveLength(1);
		await r.call.hold(true);
		r.ackAudio();
		await tick();
		expect(r.sent.filter((one) => one.cmd === "voice.audio")).toHaveLength(1);
		expect(r.sent.some((one) => one.params.final === true)).toBe(false);
		expect(r.call.current.phase).toBe("held");
		await r.call.hold(false);
		expect(r.call.current.phase).toBe("listening");
		r.call.hangUp();
	});

	test("hanging up while resuming cannot revive the call clock", async () => {
		const r = rig();
		await r.call.start();
		await r.call.hold(true);
		r.holdResume();
		const resuming = r.call.hold(false);
		r.call.hangUp();
		r.releaseResume();
		await resuming;
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.clock.since).toBeNull();
	});

	test("a temporary output gap waits for the final chunk before reopening the mic", async () => {
		const r = rig();
		await r.call.start();
		r.desk({ type: "clip", id: "s1", index: 0, final: false, mimeType: "audio/wav", data: "one" });
		r.desk({ type: "state", state: "speaking" });
		r.finishClip();
		expect(r.call.current.phase).toBe("thinking");
		r.desk({ type: "clip", id: "s1", index: 1, final: true, mimeType: "audio/wav", data: "two" });
		r.desk({ type: "state", state: "listening" });
		expect(r.call.current.phase).toBe("speaking");
		r.finishClip();
		expect(r.call.current.phase).toBe("listening");
		r.call.hangUp();
	});

	test("a failed partial TTS stream does not leave the microphone blocked after listening", async () => {
		const r = rig();
		await r.call.start();
		r.desk({ type: "state", state: "speaking" });
		r.desk({ type: "clip", id: "partial", index: 0, final: false, mimeType: "audio/wav", data: "one" });
		r.finishClip();
		expect(r.call.current.phase).toBe("thinking");
		r.desk({ type: "clip", id: "recovery", index: 0, final: true, mimeType: "audio/wav", data: "fallback" });
		r.desk({ type: "state", state: "listening" });
		expect(r.call.current.phase).toBe("speaking");
		r.finishClip();
		expect(r.call.current.phase).toBe("listening");
		r.call.hangUp();
	});

	test("an error after a partial stream lets queued audio finish and then ends", async () => {
		const r = rig();
		await r.call.start();
		r.desk({ type: "clip", id: "partial", index: 0, final: false, mimeType: "audio/wav", data: "one" });
		r.desk({ type: "state", state: "ended", reason: "error" });
		expect(r.call.current.phase).toBe("speaking");
		r.finishClip();
		expect(r.call.current.phase).toBe("ended");
	});

	test("duplicate and out-of-order output chunks cannot replay or jump ahead", async () => {
		const r = rig();
		await r.call.start();
		for (const index of [2, 0, 0, 2, 1]) r.desk({ type: "clip", id: "line", index, final: index === 1, mimeType: "audio/wav", data: String(index) });
		expect(r.played).toEqual(["0", "1"]);
		r.call.hangUp();
	});
});

/** This Mac's speech recognition, faked: what the engine reports is whatever the test emits. */
function fakeSpeech(capability: Partial<TranscriptionCapability> = {}, granted = true) {
	let onEvent: (event: TranscriptionEvent) => void = () => {};
	let final = "";
	let stopping: (() => Promise<string>) | null = null;
	const counts = { permits: 0, starts: 0, stops: 0, cancels: 0 };
	const transcription: DeviceTranscription = {
		capability: async () => ({ available: true, onDevice: true, locale: "en-US", engine: "apple-analyzer", ...capability }),
		permit: async () => {
			counts.permits++;
			return granted;
		},
		start: async (callback) => {
			onEvent = callback;
			final = "";
			counts.starts++;
			return true;
		},
		stop: async () => {
			counts.stops++;
			return stopping !== null ? stopping() : final;
		},
		cancel: async () => {
			counts.cancels++;
		},
	};
	return {
		transcription,
		counts,
		emit: (event: TranscriptionEvent) => {
			if (event.type === "final") final = event.text;
			onEvent(event);
		},
		finalize: (text: string) => (final = text),
		stopWith: (next: () => Promise<string>) => (stopping = next),
	};
}

const TEXT_DESK = ["voice", "voiceDirectCalls", "voiceTextInput"];

function textRig(speech = fakeSpeech(), response: unknown = { callId: "c", input: ["text/plain"], output: "audio/wav", inputMode: "text" }, capabilities = TEXT_DESK) {
	const r = rig({}, response, capabilities, speech.transcription);
	return {
		...r,
		speech,
		/** Native levels in dBFS every 50ms, as the engine meters the microphone. */
		levels: (db: number, fromMs: number, toMs: number) => {
			for (let at = fromMs; at <= toMs; at += 50) {
				r.at(at);
				speech.emit({ type: "level", levelDb: db, at, unit: "dbfs" });
			}
		},
		sentText: () => r.sent.filter((one) => one.cmd === "voice.text").map((one) => one.params),
	};
}

describe("a call heard by this Mac", () => {
	test("runs as text when the desk, this Mac and the person all allow it, and leaves the webview mic shut", async () => {
		const r = textRig();
		await r.call.start();
		expect(r.sent.find((one) => one.cmd === "voice.call_start")?.params.inputMode).toBe("text");
		expect(r.speech.counts.permits).toBe(1);
		expect(r.mic.opened).toBe(0);
		expect(r.call.current.phase).toBe("listening");
		expect(r.speech.counts.starts).toBe(1);
	});

	test("a finished utterance is stopped for its whole text and sent with a rising seq", async () => {
		const r = textRig();
		await r.call.start();
		// Words before any voice on the meter are not shown.
		r.at(1_150);
		r.speech.emit({ type: "partial", text: "call" });
		expect(r.call.current.lines).toEqual([]);
		r.levels(-20, 1_200, 1_400);
		expect(r.call.current.phase).toBe("hearing");
		r.speech.emit({ type: "partial", text: "call mack" });
		expect(r.call.current.lines).toEqual([{ kind: "you", id: "heard-1", text: "call mack" }]);
		r.speech.finalize("Call Mack and ask about the build.");
		r.levels(-60, 1_450, 2_700);
		expect(r.call.current.phase).toBe("thinking");
		await tick();
		expect(r.sentText()).toEqual([{ callId: r.call.id, seq: 1, text: "Call Mack and ask about the build." }]);
		expect(r.call.current.lines).toEqual([{ kind: "you", id: "heard-1", text: "Call Mack and ask about the build." }]);
		expect(r.sent.some((one) => one.cmd === "voice.audio" || one.cmd === "voice.utterance")).toBe(false);

		r.desk({ type: "heard", seq: 1, text: "Call Mack and ask about the build." });
		r.desk({ type: "state", state: "listening" });
		expect(r.speech.counts.starts).toBe(2);
		r.speech.finalize("Thanks.");
		r.levels(-20, 3_000, 3_300);
		r.levels(-60, 3_350, 4_600);
		await tick();
		expect(r.sentText().map((one) => one.seq)).toEqual([1, 2]);
	});

	test("words the room said before the speaker's onset are not sent", async () => {
		const r = textRig();
		await r.call.start();
		r.at(1_200);
		r.speech.emit({ type: "partial", text: "the radio says" });
		r.levels(-60, 1_250, 2_000);
		r.levels(-20, 2_050, 2_300);
		r.speech.emit({ type: "partial", text: "the radio says call Mack" });
		expect(r.call.current.lines).toEqual([{ kind: "you", id: "heard-1", text: "call Mack" }]);
		r.speech.finalize("The radio says call Mack.");
		r.levels(-60, 2_350, 3_600);
		await tick();
		expect(r.sentText().map((one) => one.text)).toEqual(["call Mack."]);
	});

	test("a cough with no words sends nothing and listens again", async () => {
		const r = textRig();
		await r.call.start();
		r.levels(-20, 1_200, 1_400);
		r.levels(-60, 1_450, 2_700);
		await tick();
		expect(r.sentText()).toEqual([]);
		expect(r.call.current.phase).toBe("listening");
		expect(r.speech.counts.starts).toBe(2);
	});

	test("the engine lets the microphone go while the desk speaks, so it never hears it", async () => {
		const r = textRig();
		await r.call.start();
		const cancels = r.speech.counts.cancels;
		r.desk({ type: "said", id: "s1", text: "On it." });
		r.desk({ type: "clip", id: "s1", index: 0, final: true, mimeType: "audio/wav", data: "one" });
		expect(r.call.current.phase).toBe("speaking");
		expect(r.speech.counts.cancels).toBe(cancels + 1);
		// Late words from the cancelled session are not the speaker's.
		r.speech.emit({ type: "partial", text: "on it" });
		r.levels(-20, 1_200, 1_400);
		expect(r.call.current.lines.filter((one) => one.kind === "you")).toEqual([]);
		r.finishClip();
		expect(r.call.current.phase).toBe("listening");
		expect(r.speech.counts.starts).toBe(2);
	});

	test("holding while the engine finishes discards those words, and resuming listens on this Mac again", async () => {
		const r = textRig();
		await r.call.start();
		let finish: (text: string) => void = () => {};
		r.speech.stopWith(() => new Promise((resolve) => (finish = resolve)));
		r.levels(-20, 1_200, 1_400);
		r.speech.emit({ type: "partial", text: "never mind" });
		r.levels(-60, 1_450, 2_700);
		await r.call.hold(true);
		expect(r.call.current.phase).toBe("held");
		expect(r.call.current.lines).toEqual([]);
		finish("Never mind.");
		await tick();
		expect(r.sentText()).toEqual([]);
		await r.call.hold(false);
		expect(r.call.current.phase).toBe("listening");
		expect(r.mic.opened).toBe(0);
		expect(r.speech.counts.starts).toBe(2);
	});

	test("cutting in on the desk listens on this Mac again", async () => {
		const r = textRig();
		await r.call.start();
		r.desk({ type: "state", state: "thinking" });
		r.call.interrupt();
		r.desk({ type: "state", state: "listening" });
		expect(r.call.current.phase).toBe("listening");
		expect(r.speech.counts.starts).toBe(2);
	});

	test("a turn the engine split at a pause is sent whole", async () => {
		const r = textRig();
		await r.call.start();
		r.levels(-20, 1_200, 1_400);
		r.speech.emit({ type: "final", text: "Call Mack" });
		r.speech.emit({ type: "ended", reason: "final" });
		expect(r.speech.counts.starts).toBe(2);
		r.speech.emit({ type: "partial", text: "about the build" });
		expect(r.call.current.lines).toEqual([{ kind: "you", id: "heard-1", text: "Call Mack about the build" }]);
		r.speech.finalize("about the build.");
		r.levels(-60, 1_450, 2_700);
		await tick();
		expect(r.sentText().map((one) => one.text)).toEqual(["Call Mack about the build."]);
	});

	for (const [why, capabilities, speech] of [
		["the desk takes no text", ["voice", "voiceDirectCalls"], fakeSpeech()],
		["this Mac cannot recognize speech", TEXT_DESK, fakeSpeech({ available: false, reason: "No speech here" })],
		["the person does not allow it", TEXT_DESK, fakeSpeech({}, false)],
	] as const) {
		test(`stays on audio when ${why}`, async () => {
			const r = textRig(speech, { input: ["audio/wav"] }, [...capabilities]);
			await r.call.start();
			const start = r.sent.find((one) => one.cmd === "voice.call_start");
			expect(start?.params.inputMode).toBeUndefined();
			expect(r.mic.opened).toBe(1);
			expect(speech.counts.starts).toBe(0);
			r.speak(0.3, 1_000, 1_600);
			r.speak(0, 1_650, 3_000);
			expect(r.sent.filter((one) => one.cmd === "voice.utterance")).toHaveLength(1);
		});
	}

	test("asks the desk about a text call, which needs a voice but no transcription provider", async () => {
		const r = textRig();
		await r.call.start();
		expect(r.sent.find((one) => one.cmd === "voice.status")?.params).toEqual({ inputMode: "text" });
	});

	test("a Mac that cannot hear asks about an audio call, as before", async () => {
		const r = textRig(fakeSpeech({ available: false, reason: "No speech here" }), { input: ["audio/wav"] });
		await r.call.start();
		expect(r.sent.find((one) => one.cmd === "voice.status")?.params).toEqual({});
	});

	test("a person who picked a provider for this Mac keeps calls on audio, and is never asked for speech", async () => {
		const stored = new Map<string, string>([["hotline.hearOnThisMac", "off"]]);
		const before = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
		Object.defineProperty(globalThis, "localStorage", {
			value: { getItem: (key: string) => stored.get(key) ?? null, setItem: (key: string, value: string) => stored.set(key, value) },
			configurable: true,
		});
		try {
			const speech = fakeSpeech();
			const r = textRig(speech, { input: ["audio/wav"] });
			await r.call.start();
			expect(r.sent.find((one) => one.cmd === "voice.status")?.params).toEqual({});
			expect(r.sent.find((one) => one.cmd === "voice.call_start")?.params.inputMode).toBeUndefined();
			expect(speech.counts.permits).toBe(0);
			expect(r.mic.opened).toBe(1);
		} finally {
			if (before) Object.defineProperty(globalThis, "localStorage", before);
			else Reflect.deleteProperty(globalThis, "localStorage");
		}
	});

	test("the permission is not asked when the desk takes no text", async () => {
		const speech = fakeSpeech();
		const r = textRig(speech, null, ["voice", "voiceDirectCalls"]);
		await r.call.start();
		expect(speech.counts.permits).toBe(0);
	});

	test("a desk that does not accept the text it was offered ends the call plainly", async () => {
		const r = textRig(fakeSpeech(), { input: ["audio/wav"] });
		await r.call.start();
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.trouble).toBe("The desk did not accept on-device transcription. Update it and call again.");
		expect(r.sent.at(-1)?.cmd).toBe("voice.call_end");
	});
});
