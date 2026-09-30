import { describe, expect, test } from "bun:test";
import {
	IDLE_DROP_MS,
	isGoodbye,
	levelFromDb,
	MAX_TURN_MS,
	MIN_SPEECH_MS,
	SILENCE_MS,
	TurnDetector,
	type TurnEvent,
} from "../src/voice/turn";

const SPEECH = 0.1;
const QUIET = 0.004;

/** A meter reporting `level` from `from` to `to`, every `every` ms. */
function run(detector: TurnDetector, level: number, from: number, to: number, every = 16): TurnEvent[] {
	const events: TurnEvent[] = [];
	for (let now = from; now < to; now += every) events.push(...detector.push(level, now));
	return events;
}

describe("a turn", () => {
	test("starts once the voice outlasts a blip, and ends after 700ms of quiet", () => {
		const detector = new TurnDetector(0);
		const talking = run(detector, SPEECH, 0, 1500);
		expect(talking.map((event) => event.kind)).toEqual(["start"]);
		expect(talking[0]).toEqual({ kind: "start", at: 0 });

		const after = run(detector, QUIET, 1500, 3000);
		expect(after).toHaveLength(1);
		const end = after[0]!;
		expect(end.kind).toBe("end");
		expect(end.kind === "end" && end.reason).toBe("silence");
		// The last voiced sample was at 1488, so the end lands just past 700ms after it.
		expect(end.at).toBeGreaterThan(1488 + SILENCE_MS);
		expect(end.at).toBeLessThanOrEqual(1488 + SILENCE_MS + 32);
	});

	test("ignores a blip shorter than 250ms, and the clip carries on", () => {
		const detector = new TurnDetector(0);
		const events = [
			...run(detector, QUIET, 0, 1000),
			...run(detector, SPEECH, 1000, 1000 + MIN_SPEECH_MS - 16),
			...run(detector, QUIET, 1000 + MIN_SPEECH_MS - 16, 4000),
		];
		expect(events).toEqual([]);
	});

	test("a blip in a silent clip does not keep the clip from being dropped", () => {
		const detector = new TurnDetector(0);
		const events = [
			...run(detector, SPEECH, 1000, 1100),
			...run(detector, QUIET, 1100, IDLE_DROP_MS + 2000),
		];
		expect(events.map((event) => event.kind)).toEqual(["drop"]);
	});

	test("a pause shorter than 700ms does not split a sentence", () => {
		const detector = new TurnDetector(0);
		const events = [
			...run(detector, SPEECH, 0, 800),
			...run(detector, QUIET, 800, 800 + SILENCE_MS - 100),
			...run(detector, SPEECH, 800 + SILENCE_MS - 100, 2500),
		];
		expect(events.map((event) => event.kind)).toEqual(["start"]);
	});

	test("is cut at 20 seconds however long the person keeps talking", () => {
		const detector = new TurnDetector(0);
		const events = run(detector, SPEECH, 0, MAX_TURN_MS + 1000);
		expect(events.map((event) => event.kind)).toEqual(["start", "end"]);
		const end = events[1]!;
		expect(end.kind === "end" && end.reason).toBe("max");
		expect(end.at).toBeGreaterThan(MAX_TURN_MS);
		expect(end.at).toBeLessThanOrEqual(MAX_TURN_MS + 16);
	});

	test("counts the 20 seconds from the start of the clip, silence included", () => {
		const detector = new TurnDetector(0);
		const events = [
			...run(detector, QUIET, 0, 7000),
			...run(detector, SPEECH, 7000, MAX_TURN_MS + 1000),
		];
		expect(events.map((event) => event.kind)).toEqual(["start", "end"]);
		expect(events[1]!.at).toBeLessThanOrEqual(MAX_TURN_MS + 16);
	});

	test("ignores everything after it ends, until the client resets it", () => {
		const detector = new TurnDetector(0);
		run(detector, SPEECH, 0, 1000);
		run(detector, QUIET, 1000, 2000);
		expect(run(detector, SPEECH, 2000, 3000)).toEqual([]);

		detector.reset(3000);
		const again = run(detector, SPEECH, 3000, 4000);
		expect(again.map((event) => event.kind)).toEqual(["start"]);
		expect(again[0]).toEqual({ kind: "start", at: 3000 });
	});

	test("starts from a clock that reads zero", () => {
		const detector = new TurnDetector(0);
		expect(run(detector, SPEECH, 0, 400)[0]).toEqual({ kind: "start", at: 0 });
	});
});

describe("a clip of pure silence", () => {
	test("is dropped after 8 seconds and the next one starts clean", () => {
		const detector = new TurnDetector(0);
		const first = run(detector, QUIET, 0, IDLE_DROP_MS + 100);
		expect(first).toHaveLength(1);
		expect(first[0]!.kind).toBe("drop");

		// The drop began a new clip: a whole turn follows without a reset call.
		const from = IDLE_DROP_MS + 100;
		const turn = [...run(detector, SPEECH, from, from + 1000), ...run(detector, QUIET, from + 1000, from + 2500)];
		expect(turn.map((event) => event.kind)).toEqual(["start", "end"]);
	});

	test("is not dropped under a person who begins speaking as the 8 seconds run out", () => {
		const detector = new TurnDetector(0);
		const events = [
			...run(detector, QUIET, 0, IDLE_DROP_MS - 48),
			...run(detector, SPEECH, IDLE_DROP_MS - 48, IDLE_DROP_MS + 1500),
			...run(detector, QUIET, IDLE_DROP_MS + 1500, IDLE_DROP_MS + 3000),
		];
		expect(events.map((event) => event.kind)).toEqual(["start", "end"]);
	});
});

describe("the noise floor", () => {
	test("rises to a steady room and stops counting it as voice", () => {
		const level = 0.03;
		const fresh = new TurnDetector(0);
		expect(run(fresh, level, 0, 1000).map((event) => event.kind)).toEqual(["start"]);

		// The same level, but the room has hummed at 0.014 for a while first.
		const learned = new TurnDetector(0);
		run(learned, 0.014, 0, 6000);
		expect(learned.noiseFloor).toBeCloseTo(0.014, 3);
		learned.reset(6000);
		expect(run(learned, level, 6000, 7000)).toEqual([]);
		// Speech well above the room still gets through.
		expect(run(learned, SPEECH, 7000, 8000).map((event) => event.kind)).toEqual(["start"]);
	});

	test("is learned at the same pace however often the meter reports", () => {
		const fast = new TurnDetector(0);
		const slow = new TurnDetector(0);
		run(fast, 0.012, 0, 2000, 16);
		run(slow, 0.012, 0, 2000, 100);
		expect(Math.abs(fast.noiseFloor - slow.noiseFloor)).toBeLessThan(0.0005);
	});

	test("does not learn speech as the room", () => {
		const detector = new TurnDetector(0);
		run(detector, SPEECH, 0, 5000);
		expect(detector.noiseFloor).toBeCloseTo(0.008, 5);
	});

	test("survives a meter that reports nonsense", () => {
		const detector = new TurnDetector(0);
		expect(detector.push(Number.NaN, 0)).toEqual([]);
		expect(detector.push(-1, 16)).toEqual([]);
		expect(detector.push(Number.POSITIVE_INFINITY, 32)).toEqual([]);
		expect(Number.isFinite(detector.noiseFloor)).toBe(true);
	});
});

describe("levels from decibels", () => {
	test("full scale is 1, minus 20 is a tenth, and silence or junk is 0", () => {
		expect(levelFromDb(0)).toBe(1);
		expect(levelFromDb(-20)).toBeCloseTo(0.1, 6);
		expect(levelFromDb(-160)).toBeLessThan(1e-7);
		expect(levelFromDb(6)).toBe(1);
		expect(levelFromDb(Number.NEGATIVE_INFINITY)).toBe(0);
		expect(levelFromDb(Number.NaN)).toBe(0);
	});
});

describe("goodbye", () => {
	test("hears a farewell that is the whole utterance", () => {
		for (const said of [
			"Bye",
			"OK, bye!",
			"Okay bye bye",
			"Thanks, goodbye.",
			"Alright, thank you, see you later",
			"See ya later, Hotline",
			"bye-bye desk",
		]) {
			expect(isGoodbye(said)).toBe(true);
		}
	});

	test("does not hear one inside a request", () => {
		for (const said of [
			"Say goodbye to Mack",
			"Tell Mack bye and check the PR",
			"Bye the way, where is Clem",
			"See you later today Mack should look at it",
			"",
			"Hello",
		]) {
			expect(isGoodbye(said)).toBe(false);
		}
	});
});
