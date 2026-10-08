import { describe, expect, test } from "bun:test";
import {
	DICTATION_DENIED,
	DICTATION_UNAVAILABLE,
	Dictation,
	type DictationEngine,
	type DictationEvent,
	followLevel,
	joinDictated,
	levelFromDbfs,
	macEngine,
	requestDictation,
	takeDictationRequests,
} from "../src/voice/dictation";
import type { DeviceTranscription, TranscriptionEvent } from "../src/voice/transcription";

/** An engine that does what the test says, and remembers what it was asked. */
function fakeEngine(overrides: Partial<DictationEngine> = {}) {
	const calls: string[] = [];
	let listener: ((event: DictationEvent) => void) | null = null;
	let final = "";
	const engine: DictationEngine = {
		capability: async () => ({ available: true, onDevice: true, locale: "en-US" }),
		permit: async () => {
			calls.push("permit");
			return true;
		},
		start: async (onEvent) => {
			calls.push("start");
			listener = onEvent;
			return true;
		},
		stop: async () => {
			calls.push("stop");
			return final;
		},
		cancel: async () => {
			calls.push("cancel");
		},
		...overrides,
	};
	return {
		engine,
		calls,
		emit: (event: DictationEvent) => listener?.(event),
		finalText: (text: string) => (final = text),
	};
}

function fakeField(text = "") {
	const field = { text, read: () => field.text, write: (next: string) => (field.text = next) };
	return field;
}

/** Lets the engine's promises settle. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("dictation into the composer", () => {
	test("words join what was typed before them with one space", () => {
		expect(joinDictated("", ["hello"])).toBe("hello");
		expect(joinDictated("Note:", ["hello", "there"])).toBe("Note: hello there");
		expect(joinDictated("Note: ", ["hello"])).toBe("Note: hello");
		expect(joinDictated("Note:", ["", " "])).toBe("Note:");
	});

	test("partials replace the dictated span and keep the typed text in front of it", async () => {
		const fake = fakeEngine();
		const field = fakeField("Please");
		const dictation = new Dictation(fake.engine, field);
		await dictation.start();
		expect(dictation.view.phase).toBe("listening");
		fake.emit({ type: "partial", text: "fix" });
		expect(field.text).toBe("Please fix");
		fake.emit({ type: "partial", text: "fix the build" });
		expect(field.text).toBe("Please fix the build");
	});

	test("levels go to the meter outside the view, and fall to nothing when listening ends", async () => {
		const fake = fakeEngine();
		const dictation = new Dictation(fake.engine, fakeField());
		const levels: number[] = [];
		dictation.watchLevel((level) => levels.push(level));
		let views = 0;
		dictation.watch(() => views++);
		await dictation.start();
		const before = views;
		fake.emit({ type: "level", level: 0.7 });
		fake.emit({ type: "level", level: 0.3 });
		expect(views).toBe(before);
		await dictation.stop();
		expect(levels).toEqual([0.7, 0.3, 0, 0]);
	});

	test("the phases run waiting, listening, finishing, then idle", async () => {
		const fake = fakeEngine();
		const dictation = new Dictation(fake.engine, fakeField());
		const phases: string[] = [];
		dictation.watch(() => {
			if (phases.at(-1) !== dictation.view.phase) phases.push(dictation.view.phase);
		});
		await dictation.start();
		await dictation.stop();
		expect(phases).toEqual(["starting", "listening", "finishing", "idle"]);
		// A press while it waits for the engine gives up instead.
		dictation.toggle();
		dictation.toggle();
		expect(dictation.view.phase).toBe("idle");
	});

	test("decibels become a level evenly from a quiet room to a raised voice", () => {
		expect(levelFromDbfs(-80)).toBe(0);
		expect(levelFromDbfs(-60)).toBe(0);
		expect(levelFromDbfs(-35)).toBeCloseTo(0.5);
		expect(levelFromDbfs(-10)).toBe(1);
		expect(levelFromDbfs(0)).toBe(1);
		expect(levelFromDbfs(Number.NaN)).toBe(0);
	});

	test("a meter's level rises quickly, falls slowly, and does not care about the frame rate", () => {
		const up = followLevel(0, 1, 40);
		const down = 1 - followLevel(1, 0, 40);
		expect(up).toBeGreaterThan(0.6);
		expect(down).toBeLessThan(0.2);
		// Two half frames land where one whole frame does.
		expect(followLevel(followLevel(0, 1, 8), 1, 8)).toBeCloseTo(followLevel(0, 1, 16));
		expect(followLevel(0.4, 0.4, 16)).toBe(0.4);
		expect(followLevel(0.4, 1, -5)).toBe(0.4);
	});

	test("this Mac's engine hands dictation a level, not decibels", async () => {
		let send: (event: TranscriptionEvent) => void = () => {};
		const speech: DeviceTranscription = {
			capability: async () => ({ available: true, onDevice: true, locale: "en-US" }),
			permit: async () => true,
			start: async (onEvent) => {
				send = onEvent;
				return true;
			},
			stop: async () => "",
			cancel: async () => {},
		};
		const heard: DictationEvent[] = [];
		await macEngine(speech).start((event) => heard.push(event));
		send({ type: "level", levelDb: -35, at: 1, unit: "dbfs" });
		send({ type: "partial", text: "hi" });
		expect(heard).toEqual([{ type: "level", level: 0.5 }, { type: "partial", text: "hi" }]);
	});

	test("stopping puts the engine's complete final text in the field and sends nothing", async () => {
		const fake = fakeEngine();
		const field = fakeField("Please");
		const dictation = new Dictation(fake.engine, field);
		await dictation.start();
		fake.emit({ type: "partial", text: "fix the bil" });
		fake.finalText("fix the build.");
		dictation.toggle();
		expect(dictation.view.phase).toBe("finishing");
		await settle();
		expect(field.text).toBe("Please fix the build.");
		expect(dictation.view).toEqual({ phase: "idle", error: null });
		expect(fake.calls).toEqual(["permit", "start", "stop"]);
	});

	test("cancelling puts the field back as it was", async () => {
		const fake = fakeEngine();
		const field = fakeField("Please");
		const dictation = new Dictation(fake.engine, field);
		await dictation.start();
		fake.emit({ type: "partial", text: "never mind" });
		expect(field.text).toBe("Please never mind");
		dictation.cancel();
		expect(field.text).toBe("Please");
		expect(dictation.view.phase).toBe("idle");
		expect(fake.calls.at(-1)).toBe("cancel");
		// The cancelled session's late words do not come back.
		fake.emit({ type: "partial", text: "never mind at all" });
		expect(field.text).toBe("Please");
	});

	test("a pause the engine ends an utterance on starts another session, and its words carry on after", async () => {
		const fake = fakeEngine();
		const field = fakeField();
		const dictation = new Dictation(fake.engine, field);
		await dictation.start();
		fake.emit({ type: "final", text: "First thought." });
		fake.emit({ type: "ended", reason: "final" });
		await settle();
		expect(fake.calls.filter((call) => call === "start")).toHaveLength(2);
		expect(dictation.view.phase).toBe("listening");
		// Silence with nothing said keeps listening too.
		fake.emit({ type: "ended", reason: "no-speech" });
		await settle();
		expect(fake.calls.filter((call) => call === "start")).toHaveLength(3);
		fake.emit({ type: "partial", text: "Second" });
		expect(field.text).toBe("First thought. Second");
		fake.finalText("Second thought.");
		await dictation.stop();
		expect(field.text).toBe("First thought. Second thought.");
	});

	test("permission is asked once, and a refusal says where to allow it", async () => {
		const fake = fakeEngine({ permit: async () => false });
		const field = fakeField("Draft");
		const dictation = new Dictation(fake.engine, field);
		await dictation.start();
		expect(dictation.view).toEqual({ phase: "idle", error: DICTATION_DENIED });
		expect(field.text).toBe("Draft");

		const granted = fakeEngine();
		const again = new Dictation(granted.engine, fakeField());
		await again.start();
		await again.stop();
		await again.start();
		expect(granted.calls.filter((call) => call === "permit")).toHaveLength(1);
	});

	test("a machine that cannot hear says so instead of asking", async () => {
		const fake = fakeEngine({ capability: async () => ({ available: false, onDevice: true, locale: "" }) });
		const dictation = new Dictation(fake.engine, fakeField());
		await dictation.start();
		expect(dictation.view.error).toBe(DICTATION_UNAVAILABLE);
		expect(fake.calls).not.toContain("permit");
	});

	test("an engine error stops listening, keeps what was heard and says why", async () => {
		const fake = fakeEngine();
		const field = fakeField();
		const dictation = new Dictation(fake.engine, field);
		await dictation.start();
		fake.emit({ type: "partial", text: "half a" });
		fake.emit({ type: "error", message: "The microphone changed." });
		expect(dictation.view).toEqual({ phase: "idle", error: "The microphone changed." });
		expect(field.text).toBe("half a");
	});

	test("a stop that does not finish keeps the words on screen and says why", async () => {
		const fake = fakeEngine({ stop: () => Promise.reject(new Error("Speech recognition on this Mac did not finish. Try again.")) });
		const field = fakeField();
		const dictation = new Dictation(fake.engine, field);
		await dictation.start();
		fake.emit({ type: "partial", text: "almost" });
		await dictation.stop();
		expect(field.text).toBe("almost");
		expect(dictation.view.error).toContain("did not finish");
	});

	test("a second dictation lets the first go and puts its field back", async () => {
		const fake = fakeEngine();
		const one = fakeField("one");
		const first = new Dictation(fake.engine, one);
		await first.start();
		fake.emit({ type: "partial", text: "heard" });
		const second = new Dictation(fake.engine, fakeField());
		await second.start();
		expect(first.view.phase).toBe("idle");
		expect(one.text).toBe("one");
		second.cancel();
	});

	test("the shortcut reaches the composer on screen, or the one about to mount", async () => {
		const pressed: string[] = [];
		const release = takeDictationRequests(() => pressed.push("on screen"));
		requestDictation();
		expect(pressed).toEqual(["on screen"]);
		release();

		requestDictation(1_000);
		const later = takeDictationRequests(() => pressed.push("mounted"), 1_500);
		await settle();
		expect(pressed).toEqual(["on screen", "mounted"]);
		later();

		requestDictation(1_000);
		const stale = takeDictationRequests(() => pressed.push("too late"), 9_000);
		await settle();
		expect(pressed).toEqual(["on screen", "mounted"]);
		stale();
	});
});
