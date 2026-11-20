import { describe, expect, test } from "bun:test";
Object.assign(globalThis, { requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
const { Call, supportsDirectCalls } = await import("../src/voice/call");
import { UNHEARD } from "../src/voice/transcription";
import { NO_MICROPHONE } from "../src/voice/microphone";
import { interruptBySpeaking, setInterruptBySpeaking } from "../src/voice/hearing";
import { AUDIO_SILENCE_MS, FAST_SILENCE_MS } from "../src/voice/turn";
import type { CallAudio } from "../src/voice/audio";
import type { CallOptions, CallTransport } from "../src/voice/call";
import type { DeviceTranscription, TranscriptionCapability, TranscriptionEvent } from "../src/voice/transcription";

type Handlers = { snapshot(items: unknown[]): void; event(item: unknown): void };

function rig(options: CallOptions = {}, response: unknown = null, capabilities: string[] = ["voice", "voiceDirectCalls"], transcription?: DeviceTranscription) {
	const notes: string[] = [];
	options = { note: (line) => notes.push(line), ...options };
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
	const pcm: { data: string; rate: number }[] = [];
	let playedMs = 0;
	const chimes: string[] = [];
	let queue = 0;
	let idle: () => void = () => {};
	/** The reply's loudness as the player meters it, and whether the call paused it. */
	const output = { level: 0, paused: false, pauses: 0, resumes: 0, ducked: false, pausedAt: 0 };
	const mic = { open: true, closed: 0, opened: 0 };
	/**
	 * The playback clock: the last clip of a said line begun, a ten-second clip
	 * begun when it was queued, and the time it has stood still, paused.
	 */
	const clock = { clip: null as { line: string; index: number } | null, at: 0, pausedAt: null as number | null, pausedFor: 0 };
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
		play: (_mime, data, clip) => {
			played.push(data);
			queue++;
			if (clip !== undefined) Object.assign(clock, { clip, at: now, pausedFor: 0 });
		},
		playPcm: (data, rate) => {
			pcm.push({ data, rate });
			queue++;
		},
		playedMs: () => playedMs,
		position: () => clock.clip === null ? null : { ...clock.clip, ms: (clock.pausedAt ?? now) - clock.at - clock.pausedFor, clipMs: 10_000 },
		stopPlayback: () => {
			queue = 0;
			output.paused = false;
			output.ducked = false;
			Object.assign(clock, { clip: null, pausedAt: null, pausedFor: 0 });
		},
		duckPlayback: (down) => {
			output.ducked = down;
		},
		pausePlayback: () => {
			if (!output.paused) clock.pausedAt = output.pausedAt = now;
			output.paused = true;
			output.pauses++;
		},
		resumePlayback: () => {
			if (output.paused) output.resumes++;
			if (clock.pausedAt !== null) clock.pausedFor += now - clock.pausedAt;
			clock.pausedAt = null;
			output.paused = false;
		},
		get playing() {
			return queue > 0;
		},
		outputLevel: () => (output.paused ? 0 : output.level),
		chime: (kind) => {
			chimes.push(kind);
			return 0;
		},
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
		pcm,
		heardFor: (ms: number) => (playedMs = ms),
		chimes,
		notes,
		output,
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
	test("a card settled on the call or at the desk leaves the call's pending list", async () => {
		const r = rig();
		await r.call.start();
		const card = { type: "card", personaId: "ada", requestId: "q1", kind: "human_action" };
		r.desk(card);
		r.desk(card);
		expect(r.call.current.cards).toHaveLength(1);
		r.desk({ ...card, resolved: true });
		expect(r.call.current.cards).toHaveLength(0);
	});

	test("a spoken turn is sent once, and the call waits on the desk", async () => {
		const r = rig();
		await r.call.start();
		expect(r.call.current.phase).toBe("listening");
		r.speak(0.3, 1_000, 1_600);
		expect(r.call.current.phase).toBe("hearing");
		expect(r.sent.some((one) => one.cmd === "voice.input_state" && one.params["speaking"] === true)).toBe(true);
		r.speak(0, 1_650, 3_000);
		expect(r.sent.filter((one) => one.cmd === "voice.utterance")).toHaveLength(1);
		expect(r.sent.find((one) => one.cmd === "voice.utterance")!.params["silenceMs"]).toBe(AUDIO_SILENCE_MS);
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

	test("turn by turn does not listen while a reply plays, so its echo never cuts it, and a tap does", async () => {
		const r = rig();
		await r.call.start();
		r.desk({ type: "said", id: "s1", text: "A long answer." });
		r.desk({ type: "clip", id: "s1", index: 0, final: false, mimeType: "audio/wav", data: "one" });
		expect(r.call.current.phase).toBe("speaking");
		// The speaker's own reply, loud in the microphone, is not taken for the person.
		r.speak(0.3, 1_000, 1_600);
		r.speak(0, 1_650, 3_000);
		expect(r.call.current.phase).toBe("speaking");
		expect(r.sent.filter((one) => one.cmd === "voice.interrupt" || one.cmd === "voice.input_state" || one.cmd === "voice.utterance")).toEqual([]);
		r.desk({ type: "clip", id: "s1", index: 1, final: true, mimeType: "audio/wav", data: "two" });
		expect(r.played).toEqual(["one", "two"]);
		// A tap cuts it off, and the call listens again.
		r.call.interrupt();
		expect(r.sent.filter((one) => one.cmd === "voice.interrupt")).toHaveLength(1);
		expect(r.call.current.phase).toBe("listening");
		r.speak(0.3, 3_100, 3_700);
		expect(r.call.current.phase).toBe("hearing");
	});

	test("a tap, a hold and a hang-up tell the desk where playback stood, so it can say what was heard", async () => {
		const r = rig();
		await r.call.start();
		r.desk({ type: "said", id: "s1", text: "A long answer. With a second sentence." });
		r.at(2_000);
		r.desk({ type: "clip", id: "s1", index: 0, final: false, mimeType: "audio/wav", data: "one" });
		r.at(2_700);
		r.call.interrupt();
		expect(r.sent.find((one) => one.cmd === "voice.interrupt")?.params).toEqual({ callId: r.call.id, played: { line: "s1", index: 0, ms: 700, clipMs: 10_000 } });
		// The waiting sound is no line said: nothing of a said line has begun since the cut.
		r.desk({ type: "clip", id: "waiting-2", index: 0, final: true, mimeType: "audio/wav", data: "tone" });
		await r.call.hold(true);
		expect(r.sent.find((one) => one.cmd === "voice.hold")?.params).toEqual({ callId: r.call.id, hold: true, played: { index: 0, ms: 0, clipMs: 0 } });
		await r.call.hold(false);
		expect(r.sent.filter((one) => one.cmd === "voice.hold").at(-1)?.params).toEqual({ callId: r.call.id, hold: false });
		r.desk({ type: "said", id: "s2", text: "Another answer." });
		r.desk({ type: "clip", id: "s2", index: 0, final: true, mimeType: "audio/wav", data: "two" });
		r.at(3_200);
		r.call.hangUp();
		expect(r.sent.find((one) => one.cmd === "voice.call_end")?.params).toEqual({ callId: r.call.id, played: { line: "s2", index: 0, ms: 500, clipMs: 10_000 } });
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

	test("a short drop resumes the same call without starting another", async () => {
		const r = rig();
		await r.call.start();
		r.connection("open");
		expect(r.call.current.phase).toBe("listening");
		r.connection("closed");
		expect(r.call.current.phase).toBe("reconnecting");
		r.call.settle();
		expect(r.call.current.phase).toBe("reconnecting");
		r.connection("open");
		expect(r.call.current.phase).toBe("reconnecting");
		r.snapshot([{ type: "state", state: "listening" }]);
		expect(r.call.current.phase).toBe("listening");
		r.call.hangUp();
	});

	test("a fresh snapshot of any kind ends Reconnecting, not only one carrying a state", async () => {
		const r = rig();
		await r.call.start();
		r.connection("closed");
		expect(r.call.current.phase).toBe("reconnecting");
		r.connection("open");
		r.snapshot([]);
		expect(r.call.current.phase).toBe("listening");
		r.snapshot([{ type: "card", personaId: "ada", requestId: "q1", kind: "human_action" }]);
		expect(r.call.current.phase).toBe("listening");
		r.call.hangUp();
	});

	test("a hold survives a reconnect and ends with the person's word", async () => {
		const r = rig();
		await r.call.start();
		await r.call.hold(true);
		r.connection("closed");
		expect(r.call.current.phase).toBe("reconnecting");
		r.snapshot([{ type: "state", state: "held" }]);
		expect(r.call.current.phase).toBe("held");
		await r.call.hold(false);
		expect(r.call.current.phase).toBe("listening");
	});

	test("provider recovery clears Reconnecting when it succeeds or offers Turn by turn on failure", async () => {
		const r = rig();
		await r.call.start();
		r.call.receive({ type: "state", state: "listening", reconnecting: true });
		expect(r.call.current.phase).toBe("reconnecting");
		r.call.receive({ type: "state", state: "listening" });
		expect(r.call.current.phase).toBe("listening");
		r.call.receive({ type: "state", state: "ended", liveError: "Live voice could not reconnect." });
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.turnByTurnOffered).toBe(true);
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
		await tick();
		r.call.hangUp();
		r.releaseStart();
		await starting;
		// A desk call asks first whether the desk fronts calls.
		expect(r.sent.map((one) => one.cmd)).toEqual(["voice.status", "voice.call_start", "voice.call_end"]);
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
		expect(r.sent.find((one) => one.cmd === "voice.call_start")!.params).toEqual({ callId: r.call.id, streamAudio: true });
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
		expect(r.sent.find((one) => one.cmd === "voice.call_start")!.params["personaId"]).toBe("mack");
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
		expect(r.sent.filter((one) => one.cmd === "voice.audio").every((one) => one.params["final"] === false)).toBe(true);
		r.speak(0, 1_650, 2_400); // A 900ms pause still belongs to this turn.
		expect(r.call.current.phase).toBe("hearing");
		r.speak(0.3, 2_450, 2_800);
		r.speak(0, 2_850, 4_200);
		await tick();
		const chunks = r.sent.filter((one) => one.cmd === "voice.audio");
		expect(chunks.map((one) => one.params["index"])).toEqual(chunks.map((_, index) => index));
		expect(new Set(chunks.map((one) => one.params["seq"]))).toEqual(new Set([1]));
		expect(chunks.filter((one) => one.params["final"] === true)).toHaveLength(1);
		expect(chunks.at(-1)!.params["final"]).toBe(true);
		// Only the commit says how long the pause that ended the turn was.
		expect(chunks.filter((one) => one.params["silenceMs"] !== undefined).map((one) => one.params["silenceMs"])).toEqual([AUDIO_SILENCE_MS]);
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
		expect(r.sent.some((one) => one.params["final"] === true)).toBe(false);
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

describe("a call while the teammate works", () => {
	const MACK = { target: { personaId: "mack", name: "Mack" } };
	const WORKING = { type: "state", state: "thinking", listening: true };

	test("the mic stays open, and words said then steer into the open turn", async () => {
		const r = rig(MACK, { personaId: "mack" });
		await r.call.start();
		r.speak(0.3, 1_000, 1_600);
		r.speak(0, 1_650, 3_000);
		expect(r.call.current.phase).toBe("thinking");
		// The desk takes the words, and has the floor until it has handed them over.
		r.desk({ type: "state", state: "thinking" });
		expect(r.call.current.phase).toBe("thinking");
		r.desk(WORKING);
		expect(r.call.current.phase).toBe("listening");
		r.speak(0.3, 4_000, 4_600);
		expect(r.call.current.phase).toBe("hearing");
		// Said again while the person talks, it changes nothing.
		r.desk(WORKING);
		expect(r.call.current.phase).toBe("hearing");
		r.speak(0, 4_650, 6_000);
		expect(r.sent.filter((one) => one.cmd === "voice.utterance").map((one) => one.params["seq"])).toEqual([1, 2]);
		expect(r.call.current.phase).toBe("thinking");
		// A thinking that listens, said again before the desk took the words, does not open the mic.
		r.desk(WORKING);
		expect(r.call.current.phase).toBe("thinking");
		r.desk({ type: "state", state: "thinking" });
		r.desk(WORKING);
		expect(r.call.current.phase).toBe("listening");
		r.desk({ type: "state", state: "listening" });
		expect(r.call.current.phase).toBe("listening");
		r.call.hangUp();
	});

	test("a desk that thinks without listening still has the floor", async () => {
		const r = rig(MACK, { personaId: "mack" });
		await r.call.start();
		r.desk({ type: "state", state: "thinking" });
		expect(r.call.current.phase).toBe("thinking");
		r.speak(0.3, 1_000, 1_600);
		expect(r.call.current.phase).toBe("thinking");
		r.call.hangUp();
	});

	test("what the teammate says mid-turn plays through, and the mic opens again after it", async () => {
		const r = rig(MACK, { personaId: "mack" });
		await r.call.start();
		r.desk(WORKING);
		r.desk({ type: "state", state: "speaking" });
		// Until its sound arrives the call goes on listening: the person still has the floor.
		expect(r.call.current.phase).toBe("listening");
		r.desk({ type: "said", id: "s1", text: "Looking at the logs." });
		r.desk({ type: "clip", id: "s1", index: 0, final: true, mimeType: "audio/wav", data: "logs" });
		expect(r.call.current.phase).toBe("speaking");
		r.speak(0.3, 1_000, 1_600);
		r.speak(0, 1_650, 3_000);
		expect(r.sent.some((one) => one.cmd === "voice.utterance")).toBe(false);
		r.desk(WORKING);
		r.finishClip();
		expect(r.call.current.phase).toBe("listening");
		r.speak(0.3, 3_100, 3_700);
		r.speak(0, 3_750, 5_000);
		expect(r.sent.filter((one) => one.cmd === "voice.utterance")).toHaveLength(1);
		r.call.hangUp();
	});

	/** A call on which the person asked something, and the teammate's spoken answer is playing, its reply still open. */
	async function answerPlaying() {
		const r = rig(MACK, { personaId: "mack" });
		await r.call.start();
		r.speak(0.3, 1_000, 1_600);
		r.speak(0, 1_650, 3_000);
		r.desk({ type: "state", state: "thinking" });
		r.desk(WORKING);
		r.desk({ type: "state", state: "speaking" });
		r.desk({ type: "said", id: "a1", text: "Tavares leads on goals." });
		r.desk({ type: "clip", id: "a1", index: 0, final: false, mimeType: "audio/wav", data: "goals" });
		// Nothing more of the reply is in flight: its written half is still being written, and the turn is open.
		r.desk(WORKING);
		return r;
	}
	const utterances = (r: ReturnType<typeof rig>) => r.sent.filter((one) => one.cmd === "voice.utterance").map((one) => one.params["seq"]);

	test("once the answer has played the call listens, though the reply and its turn are still open", async () => {
		const r = await answerPlaying();
		expect(r.call.current.phase).toBe("speaking");
		// While it plays, the microphone is not listened to: its own sound is never taken for the person.
		const before = r.sent.length;
		r.speak(0.3, 3_100, 3_700);
		r.speak(0, 3_750, 5_000);
		expect(r.sent.slice(before).map((one) => one.cmd)).toEqual([]);
		r.finishClip();
		expect(r.call.current.phase).toBe("listening");
		// What the person says now is heard and sent, to wait behind the open turn as its next one.
		r.speak(0.3, 5_100, 5_700);
		expect(r.call.current.phase).toBe("hearing");
		r.speak(0, 5_750, 7_000);
		expect(utterances(r)).toEqual([1, 2]);
		expect(r.call.current.phase).toBe("thinking");
		// The rest of the reply they spoke over is not played.
		r.desk({ type: "clip", id: "a1", index: 1, final: true, mimeType: "audio/wav", data: "" });
		r.desk({ type: "state", state: "thinking" });
		r.desk(WORKING);
		expect(r.played).toEqual(["goals"]);
		expect(r.call.current.phase).toBe("listening");
		r.call.hangUp();
	});

	test("more of the answer after it has played plays while the person is silent, and the call listens after it", async () => {
		const r = await answerPlaying();
		r.finishClip();
		expect(r.call.current.phase).toBe("listening");
		r.desk({ type: "state", state: "speaking" });
		expect(r.call.current.phase).toBe("listening");
		r.desk({ type: "said", id: "a1", text: "Tavares leads on goals. The table is in our chat." });
		r.desk({ type: "clip", id: "a1", index: 1, final: false, mimeType: "audio/wav", data: "pointer" });
		expect(r.played).toEqual(["goals", "pointer"]);
		expect(r.call.current.phase).toBe("speaking");
		r.desk({ type: "clip", id: "a1", index: 2, final: true, mimeType: "audio/wav", data: "" });
		r.desk(WORKING);
		r.finishClip();
		expect(r.call.current.phase).toBe("listening");
		r.call.hangUp();
	});

	test("more of the answer that comes while the person is talking does not play over them, and goes when they said something", async () => {
		const r = await answerPlaying();
		r.finishClip();
		r.speak(0.3, 3_100, 3_700);
		expect(r.call.current.phase).toBe("hearing");
		r.desk({ type: "state", state: "speaking" });
		r.desk({ type: "said", id: "a1", text: "Tavares leads on goals. The table is in our chat." });
		r.desk({ type: "clip", id: "a1", index: 1, final: false, mimeType: "audio/wav", data: "pointer" });
		expect(r.played).toEqual(["goals"]);
		expect(r.call.current.phase).toBe("hearing");
		r.speak(0, 3_750, 5_000);
		expect(utterances(r)).toEqual([1, 2]);
		r.desk({ type: "clip", id: "a1", index: 2, final: true, mimeType: "audio/wav", data: "" });
		r.desk({ type: "state", state: "thinking" });
		r.desk(WORKING);
		expect(r.played).toEqual(["goals"]);
		expect(r.call.current.phase).toBe("listening");
		r.call.hangUp();
	});

	test("more of the answer said before the desk took the person's words does not play after them", async () => {
		const r = await answerPlaying();
		r.finishClip();
		r.speak(0.3, 3_100, 3_700);
		r.speak(0, 3_750, 5_000);
		expect(utterances(r)).toEqual([1, 2]);
		r.desk({ type: "state", state: "speaking" });
		r.desk({ type: "clip", id: "a1", index: 1, final: false, mimeType: "audio/wav", data: "pointer" });
		expect(r.played).toEqual(["goals"]);
		expect(r.call.current.phase).toBe("thinking");
		r.desk({ type: "state", state: "thinking" });
		r.desk(WORKING);
		expect(r.call.current.phase).toBe("listening");
		r.call.hangUp();
	});

	test("a tap still cuts off the answer while it plays", async () => {
		const r = await answerPlaying();
		r.call.interrupt();
		expect(r.sent.filter((one) => one.cmd === "voice.interrupt")).toHaveLength(1);
		expect(r.call.current.phase).toBe("listening");
		r.desk({ type: "clip", id: "a1", index: 1, final: true, mimeType: "audio/wav", data: "late" });
		expect(r.played).toEqual(["goals"]);
		r.call.hangUp();
	});

	test("what the desk begins to say while the person talks waits, and goes when they said something", async () => {
		const r = rig(MACK, { personaId: "mack" });
		await r.call.start();
		r.desk(WORKING);
		r.speak(0.3, 1_000, 1_600);
		expect(r.call.current.phase).toBe("hearing");
		r.desk({ type: "state", state: "speaking" });
		r.desk({ type: "said", id: "s1", text: "Looking at the logs." });
		r.desk({ type: "clip", id: "s1", index: 0, final: false, mimeType: "audio/wav", data: "logs" });
		expect(r.played).toEqual([]);
		expect(r.call.current.phase).toBe("hearing");
		r.speak(0, 1_650, 3_000);
		expect(r.sent.filter((one) => one.cmd === "voice.utterance")).toHaveLength(1);
		expect(r.played).toEqual([]);
		// The rest of it, and any line said before the desk took the words, is not played either.
		r.desk({ type: "clip", id: "s1", index: 1, final: true, mimeType: "audio/wav", data: "more logs" });
		r.desk({ type: "said", id: "s2", text: "Still looking." });
		r.desk({ type: "clip", id: "s2", index: 0, final: true, mimeType: "audio/wav", data: "still" });
		expect(r.played).toEqual([]);
		r.desk({ type: "state", state: "thinking" });
		r.desk({ type: "said", id: "s3", text: "Got it, switching to staging." });
		r.desk({ type: "clip", id: "s3", index: 0, final: true, mimeType: "audio/wav", data: "staging" });
		expect(r.played).toEqual(["staging"]);
		r.call.hangUp();
	});

	test("Turn by turn waiting audio comes from the core without local filler tones", async () => {
        const speech = fakeSpeech();
        const r = textRig(speech, { personaId: "mack", input: ["text/plain"] }, TEXT_DESK, MACK);
        await r.call.start();
        r.desk(WORKING);
        r.desk({ type: "clip", id: "waiting-1", index: 0, final: true, mimeType: "audio/wav", data: "soft sound" });
        expect(r.played).toEqual(["soft sound"]);
        expect(r.chimes.filter((kind) => kind === "think")).toEqual([]);
        r.call.hangUp();
    });

	test("noise taken for the person gives the floor back, and what the desk said meanwhile is said", async () => {
		const r = textRig();
		await r.call.start();
		r.levels(-20, 1_200, 1_400);
		expect(r.call.current.phase).toBe("hearing");
		r.desk({ type: "said", id: "s1", text: "On it." });
		r.desk({ type: "clip", id: "s1", index: 0, final: true, mimeType: "audio/wav", data: "one" });
		expect(r.played).toEqual([]);
		r.levels(-74, 1_450, 2_700);
		await tick();
		expect(r.sentText()).toEqual([]);
		expect(r.played).toEqual(["one"]);
		expect(r.call.current.phase).toBe("speaking");
		r.finishClip();
		expect(r.call.current.phase).toBe("listening");
		r.call.hangUp();
	});
});

/** This Mac's speech recognition, faked: what the engine reports is whatever the test emits. */
function fakeSpeech(capability: Partial<TranscriptionCapability> = {}, granted = true) {
	let onEvent: (event: TranscriptionEvent) => void = () => {};
	let final = "";
	let stopping: (() => Promise<string>) | null = null;
	let starting: (() => Promise<boolean>) | null = null;
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
			return starting !== null ? starting() : true;
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
		startWith: (next: () => Promise<boolean>) => (starting = next),
	};
}

const TEXT_DESK = ["voice", "voiceDirectCalls", "voiceTextInput"];

function textRig(speech = fakeSpeech(), response: unknown = { callId: "c", input: ["text/plain"], output: "audio/wav", inputMode: "text" }, capabilities = TEXT_DESK, options: CallOptions = {}) {
	const r = rig(options, response, capabilities, speech.transcription);
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
	test("with no microphone the engine's refusal is what the call says, and it ends rather than crashing", async () => {
		const speech = fakeSpeech();
		speech.startWith(() => Promise.reject(new Error(NO_MICROPHONE)));
		const r = textRig(speech);
		await r.call.start();
		await tick();
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.trouble).toBe(NO_MICROPHONE);
		expect(r.sent.some((one) => one.cmd === "voice.call_end")).toBe(true);
	});

	test("a start refused with no reason still says to check speech access", async () => {
		const speech = fakeSpeech();
		speech.startWith(async () => false);
		const r = textRig(speech);
		await r.call.start();
		await tick();
		expect(r.call.current.trouble).toContain("Speech recognition on this Mac could not start");
	});

	test("a microphone lost mid-call ends it with the engine's words", async () => {
		const r = textRig();
		await r.call.start();
		r.levels(-74, 1_000, 1_400);
		r.speech.emit({ type: "error", message: "Hotline can't hear the microphone any more. Call again when it's back." });
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.trouble).toBe("Hotline can't hear the microphone any more. Call again when it's back.");
	});

	test("runs as text when the desk, this Mac and the person all allow it, and leaves the webview mic shut", async () => {
		const r = textRig();
		await r.call.start();
		expect(r.sent.find((one) => one.cmd === "voice.call_start")?.params["inputMode"]).toBe("text");
		expect(r.speech.counts.permits).toBe(1);
		expect(r.mic.opened).toBe(0);
		expect(r.call.current.phase).toBe("listening");
		expect(r.speech.counts.starts).toBe(1);
	});

	test("the first sentence of a call, at a normal voice in a quiet room, is heard and sent", async () => {
		const r = textRig();
		await r.call.start();
		// Measured on a Mac with voice processing: the room near -74 dBFS, speech near -46.
		r.levels(-74, 1_000, 1_600);
		r.at(1_620);
		r.speech.emit({ type: "partial", text: "can" });
		r.levels(-46, 1_650, 1_750);
		r.speech.emit({ type: "partial", text: "can you hear" });
		r.levels(-44, 1_800, 2_300);
		r.speech.emit({ type: "partial", text: "can you hear me" });
		r.speech.finalize("Can you hear me?");
		r.levels(-74, 2_350, 3_800);
		await tick();
		expect(r.sentText().map((one) => one["text"])).toEqual(["Can you hear me?"]);
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
		r.levels(-74, 1_450, 2_700);
		expect(r.call.current.phase).toBe("thinking");
		await tick();
		// The pause that ended the turn goes with it, for the desk's timing line: a plainly finished English sentence.
		expect(r.sentText()).toEqual([{ callId: r.call.id, seq: 1, text: "Call Mack and ask about the build.", silenceMs: FAST_SILENCE_MS }]);
		expect(r.call.current.lines).toEqual([{ kind: "you", id: "heard-1", text: "Call Mack and ask about the build." }]);
		expect(r.sent.some((one) => one.cmd === "voice.audio" || one.cmd === "voice.utterance")).toBe(false);

		r.desk({ type: "heard", seq: 1, text: "Call Mack and ask about the build." });
		r.desk({ type: "state", state: "listening" });
		expect(r.speech.counts.starts).toBe(2);
		r.speech.finalize("Thanks.");
		r.levels(-20, 3_000, 3_300);
		r.levels(-74, 3_350, 4_600);
		await tick();
		expect(r.sentText().map((one) => one["seq"])).toEqual([1, 2]);
	});

	test("words the room said before the speaker's onset are not sent", async () => {
		const r = textRig();
		await r.call.start();
		r.at(1_200);
		r.speech.emit({ type: "partial", text: "the radio says" });
		r.levels(-74, 1_250, 2_000);
		r.levels(-20, 2_050, 2_300);
		r.speech.emit({ type: "partial", text: "the radio says call Mack" });
		expect(r.call.current.lines).toEqual([{ kind: "you", id: "heard-1", text: "call Mack" }]);
		r.speech.finalize("The radio says call Mack.");
		r.levels(-74, 2_350, 3_600);
		await tick();
		expect(r.sentText().map((one) => one["text"])).toEqual(["call Mack."]);
	});

	test("a cough with no words sends nothing and listens again", async () => {
		const r = textRig();
		await r.call.start();
		r.levels(-20, 1_200, 1_400);
		r.levels(-74, 1_450, 2_700);
		await tick();
		expect(r.sentText()).toEqual([]);
		expect(r.call.current.phase).toBe("listening");
		expect(r.speech.counts.starts).toBe(2);
	});

	test("with Interrupt by speaking off, this Mac stops listening while a reply plays and listens again once it ends", async () => {
		const r = textRig(undefined, undefined, undefined, { interruptBySpeaking: () => false });
		await r.call.start();
		const starts = r.speech.counts.starts;
		const cancels = r.speech.counts.cancels;
		r.desk({ type: "said", id: "s1", text: "On it." });
		r.desk({ type: "clip", id: "s1", index: 0, final: true, mimeType: "audio/wav", data: "one" });
		expect(r.call.current.phase).toBe("speaking");
		expect(r.speech.counts.cancels).toBe(cancels + 1);
		r.levels(-20, 1_200, 1_500);
		expect(r.call.current.phase).toBe("speaking");
		expect(r.sent.filter((one) => one.cmd === "voice.interrupt")).toHaveLength(0);
		r.finishClip();
		expect(r.call.current.phase).toBe("listening");
		expect(r.speech.counts.starts).toBe(starts + 1);
	});

	test("holding while the engine finishes discards those words, and resuming listens on this Mac again", async () => {
		const r = textRig();
		await r.call.start();
		let finish: (text: string) => void = () => {};
		r.speech.stopWith(() => new Promise((resolve) => (finish = resolve)));
		r.levels(-20, 1_200, 1_400);
		r.speech.emit({ type: "partial", text: "never mind" });
		r.levels(-74, 1_450, 2_700);
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
		r.levels(-74, 1_450, 2_700);
		await tick();
		expect(r.sentText().map((one) => one["text"])).toEqual(["Call Mack about the build."]);
	});

	for (const [locale, draft, early] of [
		["en-US", "Check the build", true],
		["en-US", "Okay", false],
		["en-US", "I want you to look at the failing test", false],
		["de-DE", "Check the build", false],
	] as const) {
		test(`a pause after "${draft}" heard in ${locale} ${early ? "ends the turn sooner" : "waits the full pause"}`, async () => {
			const r = textRig(fakeSpeech({ locale }));
			await r.call.start();
			r.levels(-20, 1_200, 1_400);
			r.speech.emit({ type: "partial", text: draft });
			r.speech.finalize(draft);
			// Past the short pause, short of the full one.
			r.levels(-74, 1_450, 2_200);
			await tick();
			expect(r.sentText().map((one) => one["text"])).toEqual(early ? [draft] : []);
			r.levels(-74, 2_250, 2_700);
			await tick();
			expect(r.sentText().map((one) => one["text"])).toEqual([draft]);
		});
	}

	for (const [why, capabilities, speech] of [
		["the desk takes no text", ["voice", "voiceDirectCalls"], fakeSpeech()],
		["this Mac cannot recognize speech", TEXT_DESK, fakeSpeech({ available: false, reason: "No speech here" })],
		["the person does not allow it", TEXT_DESK, fakeSpeech({}, false)],
	] as const) {
		test(`stays on audio when ${why}`, async () => {
			const r = textRig(speech, { input: ["audio/wav"] }, [...capabilities]);
			await r.call.start();
			const start = r.sent.find((one) => one.cmd === "voice.call_start");
			expect(start?.params["inputMode"]).toBeUndefined();
			expect(r.mic.opened).toBe(1);
			expect(speech.counts.starts).toBe(0);
			r.speak(0.3, 1_000, 1_600);
			r.speak(0, 1_650, 3_000);
			expect(r.sent.filter((one) => one.cmd === "voice.utterance")).toHaveLength(1);
		});
	}

	test("a voice this Mac never turns into words ends the call saying so, and offers the desk's own model", async () => {
		const r = textRig();
		r.status({ available: true, capabilities: TEXT_DESK, stt: { providerId: "local", modelId: "parakeet" } });
		await r.call.start();
		r.levels(-20, 1_200, 2_400);
		r.levels(-74, 2_450, 3_700);
		await tick();
		// One sentence is not enough to say so: it could have been a mumble.
		expect(r.call.current.phase).toBe("listening");
		r.levels(-20, 4_000, 6_200);
		r.levels(-74, 6_250, 7_500);
		await tick();
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.trouble).toBe(UNHEARD);
		expect(r.call.current.deskModel).toBe(true);
		expect(r.sent.filter((one) => one.cmd === "voice.status").at(-1)?.params).toEqual({});
	});

	test("offers no desk model when the desk would hear the call with a paid provider", async () => {
		const r = textRig();
		r.status({ available: true, capabilities: TEXT_DESK, stt: { providerId: "openai", modelId: "whisper-1" } });
		await r.call.start();
		r.levels(-20, 1_200, 4_600);
		r.levels(-74, 4_650, 5_900);
		await tick();
		expect(r.call.current.trouble).toBe(UNHEARD);
		expect(r.call.current.deskModel).toBeUndefined();
	});

	test("a quiet room, however long, is not taken for recognition that hears nothing", async () => {
		const r = textRig();
		await r.call.start();
		r.levels(-74, 1_000, 40_000);
		await tick();
		expect(r.call.current.phase).toBe("listening");
		expect(r.call.current.trouble).toBeUndefined();
	});

	test("once this Mac has heard a word, a long wordless stretch is not trouble", async () => {
		const r = textRig();
		await r.call.start();
		r.levels(-20, 1_200, 1_600);
		r.speech.emit({ type: "partial", text: "hello" });
		r.speech.finalize("Hello.");
		r.levels(-74, 1_650, 2_900);
		await tick();
		r.desk({ type: "state", state: "listening" });
		r.levels(-20, 3_000, 7_000);
		r.levels(-74, 7_050, 8_300);
		await tick();
		expect(r.call.current.trouble).toBeUndefined();
	});

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
			expect(r.sent.find((one) => one.cmd === "voice.call_start")?.params["inputMode"]).toBeUndefined();
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

describe("a call through a voice front", () => {
	const FRONT = { inputRate: 16_000, outputRate: 24_000 };
	const fronted = (front: unknown = FRONT, options: CallOptions = {}) =>
		rig(options, { callId: "c", input: ["audio/pcm"], output: "audio/wav", inputMode: "audio", ...(options.target ? { personaId: options.target.personaId } : {}), ...(front ? { front } : {}) }, ["voiceDirectCalls", "voiceFront"]);

	test("asks for the front, streams the microphone all the time and plays the front as it speaks", async () => {
		const r = fronted();
		await r.call.start();
		expect(r.sent.find((one) => one.cmd === "voice.call_start")?.params).toMatchObject({ mode: "front", inputRate: 16_000 });
		r.speak(0, 1_000, 1_200);
		const frames = r.sent.filter((one) => one.cmd === "voice.front_audio");
		expect(frames.length).toBeGreaterThan(0);
		expect(frames.map((one) => one.params["seq"])).toEqual(frames.map((_, i) => i));
		expect(r.sent.filter((one) => one.cmd === "voice.utterance" || one.cmd === "voice.audio")).toHaveLength(0);
		r.desk({ type: "frontAudio", seq: 0, data: "AAAA" });
		expect(r.pcm).toEqual([{ data: "AAAA", rate: 24_000 }]);
		expect(r.call.current.phase).toBe("speaking");
		expect(r.chimes.filter((one) => one === "think")).toHaveLength(0);
	});

	test("a teammate's call asks for the front too", async () => {
		const r = fronted(FRONT, { target: { personaId: "p1", name: "Mack" } });
		await r.call.start();
		expect(r.sent.find((one) => one.cmd === "voice.call_start")?.params).toMatchObject({ personaId: "p1", mode: "front" });
	});

	test("talking over the front stops it and tells the desk how much was heard", async () => {
		const r = fronted();
		await r.call.start();
		r.desk({ type: "frontAudio", seq: 0, data: "AAAA" });
		r.heardFor(420);
		r.desk({ type: "frontStop" });
		expect(r.sent.find((one) => one.cmd === "voice.front_barge")?.params).toMatchObject({ playedMs: 420 });
		expect(r.call.current.phase).toBe("listening");
	});

	test("local speech stops the front before a provider event and rejects queued audio", async () => {
		const r = fronted();
		await r.call.start();
		r.desk({ type: "frontAudio", seq: 0, data: "AAAA" });
		r.heardFor(120);
		r.speak(0.3, 1_000, 1_600);
		expect(r.sent.filter((one) => one.cmd === "voice.front_barge")).toHaveLength(1);
		r.desk({ type: "frontAudio", seq: 1, data: "late" });
		expect(r.pcm).toHaveLength(1);
		r.desk({ type: "heard", seq: 1, text: "Actually, the beacon." });
		r.desk({ type: "frontAudio", seq: 2, data: "new" });
		expect(r.pcm).toHaveLength(2);
	});

	test("a front's speech is heard again after a hold or a reconnect that followed a barge-in", async () => {
		const r = fronted();
		await r.call.start();
		r.desk({ type: "frontAudio", seq: 0, data: "one" });
		r.desk({ type: "frontStop" });
		r.desk({ type: "frontAudio", seq: 1, data: "dropped" });
		expect(r.pcm).toHaveLength(1);
		await r.call.hold(true);
		await r.call.hold(false);
		r.desk({ type: "frontAudio", seq: 2, data: "after the hold" });
		expect(r.pcm.map((one) => one.data)).toEqual(["one", "after the hold"]);
		r.desk({ type: "frontStop" });
		r.connection("closed");
		r.connection("open");
		r.snapshot([{ type: "state", state: "listening" }]);
		r.desk({ type: "frontAudio", seq: 3, data: "after the reconnect" });
		expect(r.pcm.map((one) => one.data)).toEqual(["one", "after the hold", "after the reconnect"]);
	});

	test("Stop barges in on the front without the pipeline's interrupt", async () => {
		const r = fronted();
		await r.call.start();
		r.desk({ type: "frontAudio", seq: 0, data: "AAAA" });
		r.call.interrupt();
		expect(r.sent.some((one) => one.cmd === "voice.front_barge")).toBe(true);
		expect(r.sent.some((one) => one.cmd === "voice.interrupt")).toBe(false);
	});

	test("the teammate working keeps the front's microphone open", async () => {
		const r = fronted();
		await r.call.start();
		r.desk({ type: "state", state: "thinking", listening: true });
		expect(r.call.current.phase).toBe("thinking");
		const before = r.sent.filter((one) => one.cmd === "voice.front_audio").length;
		r.speak(0.3, 1_000, 1_200);
		expect(r.sent.filter((one) => one.cmd === "voice.front_audio").length).toBeGreaterThan(before);
	});

	test("a front that goes mid-call ends without sending Turn by turn audio", async () => {
		const r = fronted();
		await r.call.start();
		r.desk({ type: "frontEnded" });
		expect(r.call.current.trouble).toContain("could not reconnect");
		expect(r.call.current.phase).toBe("ended");
		expect(r.call.current.turnByTurnOffered).toBe(true);
		const sent = r.sent.filter((one) => one.cmd === "voice.front_audio").length;
		r.speak(0.3, 2_000, 2_600);
		r.speak(0, 2_650, 4_000);
		expect(r.sent.filter((one) => one.cmd === "voice.front_audio")).toHaveLength(sent);
		expect(r.sent.filter((one) => one.cmd === "voice.audio")).toHaveLength(0);
	});

	test("a desk that answers without the requested front is rejected", async () => {
		const r = fronted(null);
		await r.call.start();
		r.speak(0.3, 1_000, 1_600);
		r.speak(0, 1_650, 3_000);
		expect(r.sent.filter((one) => one.cmd === "voice.front_audio")).toHaveLength(0);
		expect(r.sent.filter((one) => one.cmd === "voice.audio")).toHaveLength(0);
	});
});


test("a selected live model is requested even when the front is unavailable", async () => {
    const r = rig();
    r.status({ liveSelected: true, available: false, capabilities: ["voiceDirectCalls"], unavailable: "Live voice needs an xAI sign-in." });
    await r.call.start();
    expect(r.sent.find(one => one.cmd === "voice.call_start")?.params["mode"]).toBe("front");
    expect(r.call.current.phase).toBe("ended");
    expect(r.call.current.turnByTurnOffered).toBe(true);
    expect(r.sent.some(one => one.cmd === "voice.call_start" && one.params["mode"] === "pipeline")).toBe(false);
});

test("only an explicit Turn by turn choice overrides the live selection", async () => {
    const r = rig({ turnByTurn: true });
    r.status({ liveSelected: true, available: false, capabilities: ["voiceDirectCalls"] });
    await r.call.start();
    expect(r.sent.find(one => one.cmd === "voice.call_start")?.params["mode"]).toBe("pipeline");
    expect(r.call.current.phase).toBe("listening");
    r.call.hangUp();
});

test("a failed live snapshot says why even if the failure preceded subscription", async () => {
    const r = rig({}, { front: { inputRate: 16000, outputRate: 24000 } }, ["voiceFront"]);
    await r.call.start();
    r.snapshot([{ type: "state", state: "ended", reason: "error", liveError: "Live voice provider is unreachable." }]);
    expect(r.call.current.phase).toBe("ended");
    expect(r.call.current.trouble).toBe("Live voice provider is unreachable.");
    expect(r.call.current.turnByTurnOffered).toBe(true);
});


test("an explicit Turn by turn selection wins over an available live capability", async () => {
    const r = rig();
    r.status({ liveSelected: false, available: true, capabilities: ["voiceFront", "voiceDirectCalls"] });
    await r.call.start();
    expect(r.sent.find(one => one.cmd === "voice.call_start")?.params["mode"]).toBeUndefined();
    expect(r.call.current.phase).toBe("listening");
    r.call.hangUp();
    expect(r.call.current.turnByTurnOffered).not.toBe(true);
});

test("the explicit retry after a live failure keeps its call thread", async () => {
    const r = rig({ target: { personaId: "p1", name: "Mack" } }, { personaId: "p1", front: { inputRate: 16000, outputRate: 24000 } }, ["voiceFront", "voiceDirectCalls"]);
    await r.call.start();
    r.desk({ type: "state", state: "ended", reason: "error", liveError: "Live voice disconnected." });
    expect(r.call.redial(true).target?.thread).toEqual({ kind: "call", key: r.call.id });
});


test("a failed live call finishes its fixed notice once before the Turn by turn offer", async () => {
    const r = rig();
    await r.call.start();
    r.desk({ type: "state", state: "listening", reconnecting: true });
    r.desk({ type: "said", id: "notice", text: "Choose Turn by turn." });
    r.desk({ type: "clip", id: "notice", index: 0, final: true, mimeType: "audio/wav", data: "notice" });
    r.desk({ type: "state", state: "ended", reason: "error", liveError: "The Voice budget is spent." });
    expect(r.call.current.phase).toBe("speaking");
    expect(r.played).toEqual(["notice"]);
    r.finishClip();
    expect(r.call.current.phase).toBe("ended");
    expect(r.call.current.turnByTurnOffered).toBe(true);
    expect(r.sent.filter(({ cmd }) => cmd === "voice.call_start")).toHaveLength(1);
});

describe("interrupting by talking over the teammate, heard by this Mac", () => {
	const REPLY = "I looked through the build logs and the failing step is the linker, which ran out of memory.";
	/** A Turn by turn call heard by this Mac's analyzer with Interrupt by speaking on, the reply playing. */
	async function overReply(options: CallOptions = { interruptBySpeaking: () => true }, earlier: string[] = []) {
		const r = textRig(fakeSpeech(), undefined, TEXT_DESK, options);
		await r.call.start();
		r.levels(-74, 1_000, 1_400);
		for (const [index, text] of earlier.entries()) r.desk({ type: "said", id: `earlier-${index}`, text });
		r.desk({ type: "said", id: "s1", text: REPLY });
		r.desk({ type: "clip", id: "s1", index: 0, final: false, mimeType: "audio/wav", data: "one" });
		return {
			...r,
			/** Native levels every 50 ms while the reply plays at `output`; `db` may follow it. */
			reply: (from: number, to: number, db: (output: number, at: number) => number, output: (at: number) => number = (at) => (Math.floor(at / 100) % 2 === 0 ? 0.12 : 0.04)) => {
				for (let at = from; at <= to; at += 50) {
					r.output.level = output(at);
					r.at(at);
					r.speech.emit({ type: "level", levelDb: db(r.output.paused ? 0 : r.output.level, at), at, unit: "dbfs", processed: true });
				}
			},
			commands: () => r.sent.map((one) => one.cmd).filter((cmd) => cmd !== "voice.status" && cmd !== "voice.call_start"),
		};
	}
	const ROOM = () => -74;
	/** Speech near -40 dBFS on the processed channel, -22 as a raw microphone would read it. */
	const VOICE = () => -40;

	test("words over the reply interrupt it, the cut is recorded as a tap's is, and the words are the next turn", async () => {
		const r = await overReply();
		expect(r.call.current.phase).toBe("speaking");
		expect(r.speech.counts.starts).toBe(2);
		r.reply(1_450, 2_000, ROOM);
		r.speech.emit({ type: "partial", text: "I looked through the" });
		r.reply(2_050, 2_300, VOICE);
		expect(r.output.paused).toBe(true);
		expect(r.commands()).not.toContain("voice.interrupt");
		r.speech.emit({ type: "partial", text: "I looked through the wait" });
		expect(r.call.current.phase).toBe("hearing");
		expect(r.output.paused).toBe(false);
		expect(r.commands()).toContain("voice.interrupt");
		r.speech.emit({ type: "partial", text: "I looked through the wait use the other machine" });
		expect(r.call.current.lines.at(-1)).toEqual({ kind: "you", id: "heard-1", text: "wait use the other machine" });
		// What is still to come of the cut reply is not played.
		r.desk({ type: "clip", id: "s1", index: 1, final: true, mimeType: "audio/wav", data: "two" });
		expect(r.played).toEqual(["one"]);
		r.speech.finalize("I looked through the wait. Use the other machine.");
		r.levels(-74, 2_350, 4_000);
		await tick();
		expect(r.sentText().map((one) => one["text"])).toEqual(["wait. Use the other machine."]);
		const commands = r.commands();
		expect(commands.indexOf("voice.interrupt")).toBeLessThan(commands.indexOf("voice.text"));
		// The cut is where the voice paused the reply, not where the words decided it.
		const played = r.sent.find((one) => one.cmd === "voice.interrupt")?.params["played"] as { line: string; ms: number };
		expect(played.line).toBe("s1");
		expect(played.ms).toBe(r.output.pausedAt - 1_400);
		expect(played.ms).toBeLessThan(2_300 - 1_400);
		expect(r.notes.some((line) => line.includes("paused the reply"))).toBe(true);
		expect(r.notes.some((line) => line.includes("interrupted the reply"))).toBe(true);
		// The log says why, never what was said.
		expect(r.notes.join("\n")).not.toContain("machine");
	});

	test("a cough pauses the reply and resumes it where it was, with nothing cut or sent", async () => {
		const r = await overReply();
		r.reply(1_450, 2_000, ROOM);
		r.reply(2_050, 2_300, VOICE);
		expect(r.output.paused).toBe(true);
		r.reply(2_350, 3_400, ROOM);
		expect(r.output.paused).toBe(false);
		expect(r.output.resumes).toBe(1);
		expect(r.call.current.phase).toBe("speaking");
		r.desk({ type: "clip", id: "s1", index: 1, final: true, mimeType: "audio/wav", data: "two" });
		expect(r.played).toEqual(["one", "two"]);
		expect(r.commands().filter((cmd) => cmd === "voice.interrupt" || cmd === "voice.text" || cmd === "voice.input_state")).toEqual([]);
		r.finishClip();
		r.finishClip();
		expect(r.call.current.phase).toBe("listening");
	});

	test("the reply leaking into the microphone, and its words recognized, never interrupt it", async () => {
		const r = await overReply();
		// The reply's own sound at a tenth of its strength on the processed channel, louder than speech would need.
		const leak = (output: number) => 20 * Math.log10(Math.max(output * 0.5, 1e-6)) - 18;
		r.reply(1_450, 5_000, leak);
		r.speech.emit({ type: "partial", text: "I looked through the build logs and the failing step" });
		r.reply(5_050, 7_000, leak);
		expect(r.output.pauses).toBe(0);
		expect(r.commands()).not.toContain("voice.interrupt");
		expect(r.call.current.phase).toBe("speaking");
	});

	test("an onset whose only words are the reply's own resumes it, and a second turns listening over it off for the call", async () => {
		const r = await overReply();
		r.reply(1_450, 2_000, ROOM);
		r.reply(2_050, 2_300, VOICE);
		r.speech.emit({ type: "partial", text: "the failing step is the linker" });
		r.reply(2_350, 3_400, ROOM);
		expect(r.output.paused).toBe(false);
		r.reply(3_450, 5_500, ROOM);
		r.reply(5_550, 5_800, VOICE);
		r.speech.emit({ type: "partial", text: "the failing step is the linker which ran out of memory" });
		const before = r.speech.counts.cancels;
		r.reply(5_850, 6_900, ROOM);
		expect(r.output.paused).toBe(false);
		expect(r.speech.counts.cancels).toBeGreaterThan(before);
		expect(r.notes.some((line) => line.includes("off for this call"))).toBe(true);
		r.reply(6_950, 7_600, VOICE);
		expect(r.output.pauses).toBe(2);
		expect(r.commands()).not.toContain("voice.interrupt");
		// A tap still interrupts.
		r.call.interrupt();
		expect(r.commands()).toContain("voice.interrupt");
	});

	test("words over the acknowledgement line interrupt it and are not lost", async () => {
		const r = textRig(fakeSpeech(), undefined, TEXT_DESK, { interruptBySpeaking: () => true });
		await r.call.start();
		r.levels(-74, 1_000, 1_400);
		r.speech.emit({ type: "partial", text: "check the build" });
		r.levels(-40, 1_450, 1_900);
		r.speech.finalize("Check the build.");
		r.levels(-74, 1_950, 3_000);
		await tick();
		expect(r.sentText().map((one) => one["text"])).toEqual(["Check the build."]);
		// The desk takes the words, and says the acknowledgement while the teammate's model starts.
		r.desk({ type: "state", state: "thinking" });
		r.desk({ type: "said", id: "ack-1", text: "Checking the build now." });
		r.desk({ type: "clip", id: "ack-1", index: 0, final: true, mimeType: "audio/wav", data: "ack" });
		expect(r.call.current.phase).toBe("speaking");
		for (let at = 3_050; at <= 3_600; at += 50) {
			r.output.level = 0.1;
			r.at(at);
			r.speech.emit({ type: "level", levelDb: -74, at, unit: "dbfs", processed: true });
		}
		for (let at = 3_650; at <= 3_900; at += 50) {
			r.at(at);
			r.speech.emit({ type: "level", levelDb: -40, at, unit: "dbfs", processed: true });
		}
		expect(r.output.paused).toBe(true);
		r.speech.emit({ type: "partial", text: "no wait use staging" });
		expect(r.call.current.phase).toBe("hearing");
		r.speech.finalize("No, wait, use staging.");
		r.levels(-74, 3_950, 5_500);
		await tick();
		expect(r.sentText().map((one) => one["text"])).toEqual(["Check the build.", "No, wait, use staging."]);
		expect(r.sent.filter((one) => one.cmd === "voice.interrupt")).toHaveLength(1);
	});

	/** This window's storage holding `stored` for the setting, or nothing, for the length of `run`. */
	async function withStored(stored: string | null, run: () => Promise<void>) {
		const values = new Map<string, string>(stored === null ? [] : [["hotline.interruptBySpeaking", stored]]);
		const before = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
		Object.defineProperty(globalThis, "localStorage", {
			value: { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value) },
			configurable: true,
		});
		try {
			await run();
		} finally {
			if (before) Object.defineProperty(globalThis, "localStorage", before);
			else Reflect.deleteProperty(globalThis, "localStorage");
		}
	}

	test("never set, the setting is on: this Mac listens over the reply and words over it interrupt", async () => {
		await withStored(null, async () => {
			const r = await overReply({});
			expect(r.notes.some((line) => line.endsWith(": interrupt by speaking: on"))).toBe(true);
			expect(r.speech.counts.starts).toBe(2);
			r.reply(1_450, 2_000, ROOM);
			r.reply(2_050, 2_300, VOICE);
			expect(r.output.paused).toBe(true);
			r.speech.emit({ type: "partial", text: "wait use the other machine" });
			expect(r.commands()).toContain("voice.interrupt");
			expect(r.call.current.phase).toBe("hearing");
		});
	});

	test("the switch keeps off as off and on as on; never set reads as on", async () => {
		await withStored(null, async () => {
			expect(interruptBySpeaking()).toBe(true);
			setInterruptBySpeaking(false);
			expect(localStorage.getItem("hotline.interruptBySpeaking")).toBe("off");
			expect(interruptBySpeaking()).toBe(false);
			setInterruptBySpeaking(true);
			expect(interruptBySpeaking()).toBe(true);
		});
	});

	test("switched off in this window's storage, it stays off: nothing listens over the reply", async () => {
		await withStored("off", async () => {
			const r = await overReply({});
			expect(r.speech.counts.starts).toBe(1);
			expect(r.notes).toEqual([]);
		});
	});

	test("with the setting off, a reply plays as today: nothing listens over it and only a tap interrupts", async () => {
		const r = await overReply({ interruptBySpeaking: () => false });
		expect(r.speech.counts.starts).toBe(1);
		r.reply(1_450, 2_000, ROOM);
		r.reply(2_050, 2_600, VOICE);
		r.speech.emit({ type: "partial", text: "wait use the other machine" });
		expect(r.output.pauses).toBe(0);
		expect(r.call.current.phase).toBe("speaking");
		expect(r.commands()).toEqual([]);
		expect(r.notes).toEqual([]);
		r.call.interrupt();
		expect(r.commands()).toEqual(["voice.interrupt"]);
		expect(r.call.current.phase).toBe("listening");
	});

	test("a tap while the reply is paused for the person interrupts it and lets the pause go", async () => {
		const r = await overReply();
		r.reply(1_450, 2_000, ROOM);
		r.reply(2_050, 2_300, VOICE);
		expect(r.output.paused).toBe(true);
		r.call.interrupt();
		expect(r.output.paused).toBe(false);
		expect(r.commands()).toEqual(["voice.interrupt"]);
		expect(r.call.current.phase).toBe("listening");
	});

	test("a microphone without echo cancellation is never listened over", async () => {
		const r = await overReply();
		r.at(1_450);
		r.speech.emit({ type: "level", levelDb: -40, at: 1_450, unit: "dbfs", processed: false });
		r.reply(1_500, 2_600, VOICE);
		expect(r.output.pauses).toBe(0);
		expect(r.notes.some((line) => line.includes("no echo cancellation"))).toBe(true);
	});

	test("this Mac's older recognizer is not listened over", async () => {
		const speech = fakeSpeech({ engine: "apple-recognizer" });
		const r = textRig(speech, undefined, TEXT_DESK, { interruptBySpeaking: () => true });
		await r.call.start();
		r.desk({ type: "said", id: "s1", text: REPLY });
		r.desk({ type: "clip", id: "s1", index: 0, final: false, mimeType: "audio/wav", data: "one" });
		expect(speech.counts.starts).toBe(1);
		expect(r.notes.some((line) => line.includes("older recognizer"))).toBe(true);
	});

	/** Earlier replies on the call, the everyday words a person's own words share. */
	const EARLIER = [
		"I checked the other branch and what I found is that the tests pass there, so the problem is only on main.",
		"Sure, I can look at that. The build on the other machine is about ten minutes behind, so give it a moment.",
		"Done. I pushed the fix to your branch and the checks are running now; I'll tell you what they say.",
	];

	test("after a cough resumed the reply, a short phrase whose words come 1.3 s after it began still interrupts, in a call with earlier replies", async () => {
		const r = await overReply(undefined, EARLIER);
		r.reply(1_450, 2_000, ROOM);
		// A cough: paused, then quiet, and the reply goes on.
		r.reply(2_050, 2_300, VOICE);
		expect(r.output.paused).toBe(true);
		r.reply(2_350, 3_400, ROOM);
		// It plays on turned down while the engine's words may still come, then at full volume.
		expect(r.output.paused).toBe(false);
		expect(r.output.ducked).toBe(true);
		r.speech.emit({ type: "partial", text: "Hm" });
		r.reply(3_450, 4_500, ROOM);
		expect(r.output.ducked).toBe(false);
		expect(r.commands()).not.toContain("voice.interrupt");
		// "What about the other branch?", over in 0.6 s; its words arrive 1.3 s after it began.
		r.reply(4_550, 5_150, VOICE);
		r.reply(5_200, 5_800, ROOM);
		expect(r.output.ducked).toBe(true);
		r.speech.emit({ type: "partial", text: "Hm what about the other branch" });
		expect(r.commands()).toContain("voice.interrupt");
		expect(r.call.current.lines.at(-1)).toEqual({ kind: "you", id: "heard-1", text: "what about the other branch" });
	});

	test("words still interrupt after several pauses and resumes in one session, with the engine's text cumulative and revised", async () => {
		const r = await overReply(undefined, EARLIER);
		r.reply(1_450, 2_000, ROOM);
		let at = 2_050;
		for (const heard of ["Hm", "Hm mm-hmm", "Hmm mm-hmm okay"]) {
			r.reply(at, at + 250, VOICE);
			r.reply(at + 300, at + 1_300, ROOM);
			r.speech.emit({ type: "partial", text: heard });
			r.reply(at + 1_350, at + 3_400, ROOM);
			at += 3_500;
		}
		expect(r.output.pauses).toBe(3);
		expect(r.output.paused).toBe(false);
		expect(r.commands()).not.toContain("voice.interrupt");
		r.reply(at, at + 600, VOICE);
		r.reply(at + 650, at + 1_250, ROOM);
		// The engine revises the last of what it had and adds the person's words.
		r.speech.emit({ type: "partial", text: "Hmm mm-hmm. Okay, can you check the build on main" });
		expect(r.commands()).toContain("voice.interrupt");
	});
});

