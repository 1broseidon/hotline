import { describe, expect, test } from "bun:test";
import { DESK_MICROPHONE_BUSY, DESK_MICROPHONE_DENIED, DESK_NO_MICROPHONE, type DeskSeams, PARTIAL_EVERY_MS, SESSION_SECONDS, blockDbfs, deskEngine, microphoneTrouble } from "../src/voice/desk";
import { DICTATION_DENIED, Dictation, type DictationEngine, type DictationEvent, eitherEngine, levelFromDbfs } from "../src/voice/dictation";
import { fromBase64 } from "../src/voice/wav";

/** A world the desk engine can live in: a microphone the test speaks into, a desk that answers when told, and a clock it turns. */
function world(options: { available?: boolean; micRefused?: boolean } = {}) {
	let onBlock: ((block: Float32Array, rate: number) => void) | null = null;
	const mic = { opened: 0, closed: 0 };
	const asked: number[] = [];
	const answers: { resolve(text: string): void; reject(error: Error): void }[] = [];
	let tick: (() => void) | null = null;
	let timeout: (() => void) | null = null;
	const seams: DeskSeams = {
		available: async () => options.available ?? true,
		transcribe: (wav) => {
			// What was sent: a WAV of this many 16 kHz samples.
			asked.push((fromBase64(wav).length - 44) / 2);
			return new Promise<string>((resolve, reject) => answers.push({ resolve, reject }));
		},
		microphone: () => ({
			open: async (block) => {
				if (options.micRefused) throw new Error("NotAllowedError");
				mic.opened++;
				onBlock = block;
			},
			close: () => {
				mic.closed++;
				onBlock = null;
			},
		}),
		every: (callback) => {
			tick = callback;
			return () => (tick = null);
		},
		after: (callback) => {
			timeout = callback;
			return () => (timeout = null);
		},
	};
	return {
		seams,
		mic,
		asked,
		/** `seconds` of sound at 48 kHz, as the webview's microphone hands it over in blocks. */
		speak(seconds: number, loudness = 0.1) {
			const blocks = Math.round((seconds * 48_000) / 2400);
			for (let i = 0; i < blocks; i++) onBlock?.(new Float32Array(2400).fill(loudness), 48_000);
		},
		tick: () => tick?.(),
		ticking: () => tick !== null,
		answer: (text: string) => answers.shift()!.resolve(text),
		fail: (message: string) => answers.shift()!.reject(new Error(message)),
		expire: () => timeout?.(),
	};
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("dictation heard by the desk", () => {
	test("the meter follows the microphone, and the words so far are asked for once a second, one question at a time", async () => {
		const w = world();
		const engine = deskEngine(levelFromDbfs, w.seams);
		const events: DictationEvent[] = [];
		expect(await engine.start((event) => events.push(event))).toBe(true);
		expect(w.mic.opened).toBe(1);

		w.speak(0.5);
		const levels = events.filter((event) => event.type === "level");
		expect(levels.length).toBe(10);
		// 0.1 RMS is -20 dBFS: a speaking voice, high on the meter.
		expect(levels[0]?.type === "level" && levels[0].level).toBeCloseTo(levelFromDbfs(-20), 5);
		expect(blockDbfs(new Float32Array(4))).toBe(-Infinity);

		w.tick();
		expect(w.asked).toEqual([8_000]);
		// Still waiting for that answer: no second question, however much more is said.
		w.speak(0.5);
		w.tick();
		expect(w.asked).toEqual([8_000]);
		w.answer("Ask Mack");
		await settle();
		expect(events.at(-1)).toEqual({ type: "partial", text: "Ask Mack" });
		w.tick();
		expect(w.asked).toEqual([8_000, 16_000]);
		w.answer("Ask Mack to check");
		await settle();
		// Nothing new was said: nothing new is asked.
		w.tick();
		expect(w.asked.length).toBe(2);
		expect(PARTIAL_EVERY_MS).toBe(1_000);
	});

	test("stopping closes the microphone and answers the whole clip's words", async () => {
		const w = world();
		const engine = deskEngine(levelFromDbfs, w.seams);
		await engine.start(() => {});
		w.speak(2);
		const stopped = engine.stop();
		expect(w.mic.closed).toBe(1);
		expect(w.ticking()).toBe(false);
		expect(w.asked).toEqual([32_000]);
		w.answer("  Ask Mack to check the failing PR.  ");
		expect(await stopped).toBe("Ask Mack to check the failing PR.");
	});

	test("a desk that does not answer in time is a failure, never a partial", async () => {
		const w = world();
		const engine = deskEngine(levelFromDbfs, w.seams);
		await engine.start(() => {});
		w.speak(1);
		const stopped = engine.stop();
		w.expire();
		await expect(stopped).rejects.toThrow("did not finish");
	});

	test("half a minute ends the session with its words, and the controller carries on in a fresh one", async () => {
		const w = world();
		const field = { text: "Note:", read: () => field.text, write: (next: string) => (field.text = next) };
		const dictation = new Dictation(deskEngine(levelFromDbfs, w.seams), field);
		await dictation.start();
		expect(dictation.view.phase).toBe("listening");
		w.speak(SESSION_SECONDS);
		expect(w.mic.closed).toBe(1);
		expect(w.asked).toEqual([SESSION_SECONDS * 16_000]);
		w.answer("the first half minute");
		await settle();
		await settle();
		// A fresh session listens on, after the words already heard.
		expect(w.mic.opened).toBe(2);
		expect(field.text).toBe("Note: the first half minute");
		w.speak(1);
		const stopping = dictation.stop();
		await settle();
		w.answer("and the rest.");
		await stopping;
		expect(field.text).toBe("Note: the first half minute and the rest.");
		expect(dictation.view).toEqual({ phase: "idle", error: null });
	});

	test("stopping while a full session is still being heard answers that session's words", async () => {
		const w = world();
		const engine = deskEngine(levelFromDbfs, w.seams);
		await engine.start(() => {});
		w.speak(SESSION_SECONDS);
		const stopped = engine.stop();
		expect(w.asked.length).toBe(1);
		w.answer("all of it");
		expect(await stopped).toBe("all of it");
	});

	test("a microphone that won't open says whether it is missing, busy or refused", () => {
		const named = (name: string) => Object.assign(new Error(name), { name });
		expect(microphoneTrouble(named("NotFoundError"))).toBe(DESK_NO_MICROPHONE);
		expect(microphoneTrouble(named("OverconstrainedError"))).toBe(DESK_NO_MICROPHONE);
		expect(microphoneTrouble(named("NotReadableError"))).toBe(DESK_MICROPHONE_BUSY);
		expect(microphoneTrouble(named("NotAllowedError"))).toBe(DESK_MICROPHONE_DENIED);
		expect(microphoneTrouble("anything else")).toBe(DESK_MICROPHONE_DENIED);
	});

	test("a microphone refused is a sentence, and a cancelled session's late words are dropped", async () => {
		const refused = world({ micRefused: true });
		const field = { text: "", read: () => field.text, write: (next: string) => (field.text = next) };
		const dictation = new Dictation(deskEngine(levelFromDbfs, refused.seams), field);
		await dictation.start();
		expect(dictation.view).toEqual({ phase: "idle", error: DESK_MICROPHONE_DENIED });

		const w = world();
		const engine = deskEngine(levelFromDbfs, w.seams);
		const events: DictationEvent[] = [];
		await engine.start((event) => events.push(event));
		w.speak(1);
		w.tick();
		await engine.cancel();
		expect(w.mic.closed).toBe(1);
		w.answer("too late");
		await settle();
		expect(events.some((event) => event.type === "partial")).toBe(false);
	});

	test("a desk without a model cannot dictate", async () => {
		expect(await deskEngine(levelFromDbfs, world({ available: false }).seams).capability()).toEqual({ available: false });
		expect(await deskEngine(levelFromDbfs, world().seams).capability()).toEqual({ available: true });
	});
});

describe("a Mac chooses its own engine or the desk's", () => {
	function engine(name: string, log: string[], options: { available?: boolean; permit?: boolean } = {}): DictationEngine {
		return {
			capability: async () => ({ available: options.available ?? true }),
			permit: async () => {
				log.push(`${name} permit`);
				return options.permit ?? true;
			},
			start: async () => {
				log.push(`${name} start`);
				return true;
			},
			stop: async () => {
				log.push(`${name} stop`);
				return name;
			},
			cancel: async () => {
				log.push(`${name} cancel`);
			},
		};
	}

	test("every session goes where hearing points, and each engine is asked its permission once", async () => {
		const log: string[] = [];
		let desk = false;
		const either = eitherEngine(engine("mac", log), engine("desk", log), () => desk);
		await either.start(() => {});
		expect(await either.stop()).toBe("mac");
		await either.start(() => {});
		desk = true;
		await either.start(() => {});
		expect(await either.stop()).toBe("desk");
		await either.cancel();
		expect(log).toEqual(["mac permit", "mac start", "mac stop", "mac start", "desk permit", "desk start", "desk stop", "desk cancel"]);
	});

	test("the desk is passed over while it has no model, and a Mac that is refused says where to allow it", async () => {
		const log: string[] = [];
		const either = eitherEngine(engine("mac", log, { permit: false }), engine("desk", log, { available: false }), () => true);
		expect(await either.capability()).toEqual({ available: true });
		await expect(either.start(() => {})).rejects.toThrow(DICTATION_DENIED);
		expect(log).toEqual(["mac permit"]);
		const neither = eitherEngine(engine("mac", log, { available: false }), engine("desk", log, { available: false }), () => false);
		expect(await neither.capability()).toEqual({ available: false });
	});
});
