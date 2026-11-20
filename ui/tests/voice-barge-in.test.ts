import { describe, expect, test } from "bun:test";
import { BargeIn, CONFIRM_MS, MAX_PAUSE_MS, personWords, scriptOf } from "../src/voice/bargeIn";

const REPLY = scriptOf(["I looked through the build logs and the failing step is the linker, which ran out of memory."]);
const ROOM = 0.002;
const FLOOR = 0.008;

/** Meter readings every 50 ms from `from` to `to`, while the reply plays at `output`. */
function feed(barge: BargeIn, from: number, to: number, level: (at: number) => number, output: (at: number) => number, text = "") {
	const decisions = [];
	for (let at = from; at <= to; at += 50) {
		const decision = barge.level(level(at), output(at), at, text);
		if (decision !== null) decisions.push({ at, ...decision });
	}
	return decisions;
}

const steady = (value: number) => () => value;
/** A reply's loudness rising and falling with its syllables. */
const syllables = (at: number) => (Math.floor(at / 100) % 2 === 0 ? 0.12 : 0.04);

describe("interrupting by talking over the reply", () => {
	test("a voice over the reply pauses it, and words the reply did not say make it an interruption", () => {
		const barge = new BargeIn(0, FLOOR);
		expect(feed(barge, 0, 1_000, steady(ROOM), syllables)).toEqual([]);
		const [pause] = feed(barge, 1_050, 1_400, steady(0.15), syllables, "I looked through the");
		expect(pause?.kind).toBe("pause");
		expect(barge.paused).toBe(true);
		// The engine's words come late; the first are the reply's own, heard before it paused.
		expect(barge.words("I looked through the build", 1_600, REPLY)).toBeNull();
		const cut = barge.words("I looked through the build wait use the other machine", 2_400, REPLY);
		expect(cut?.kind).toBe("interrupt");
		expect(barge.paused).toBe(false);
	});

	test("a cough pauses the reply and resumes it where it was, with nothing taken as words", () => {
		const barge = new BargeIn(0, FLOOR);
		feed(barge, 0, 1_000, steady(ROOM), syllables);
		const decisions = [
			...feed(barge, 1_050, 1_350, steady(0.2), syllables),
			...feed(barge, 1_400, 2_400, steady(ROOM), () => 0),
		];
		expect(decisions.map((one) => one.kind)).toEqual(["pause", "resume"]);
		expect(decisions[1]?.why).toContain("quiet");
	});

	test("a voice that goes on without words resumes the reply after the longest pause", () => {
		const barge = new BargeIn(0, FLOOR);
		feed(barge, 0, 1_000, steady(ROOM), syllables);
		const decisions = feed(barge, 1_050, 1_050 + MAX_PAUSE_MS + 2_000, steady(0.2), () => 0);
		// The same voice, still going, does not pause the reply again.
		expect(decisions.map((one) => one.kind)).toEqual(["pause", "resume"]);
		expect(decisions[1]?.why).toContain("no words");
	});

	test("late words still interrupt a reply resumed after a short word, while they can be the person's", () => {
		const barge = new BargeIn(0, FLOOR);
		feed(barge, 0, 1_000, steady(ROOM), syllables);
		feed(barge, 1_050, 1_300, steady(0.2), syllables);
		feed(barge, 1_350, 2_300, steady(ROOM), () => 0);
		expect(barge.paused).toBe(false);
		expect(barge.words("stop", 2_500, REPLY)?.kind).toBe("interrupt");
	});

	test("words after the window for them are not an interruption", () => {
		const barge = new BargeIn(0, FLOOR);
		feed(barge, 0, 1_000, steady(ROOM), syllables);
		feed(barge, 1_050, 1_300, steady(0.2), syllables);
		feed(barge, 1_350, 1_050 + CONFIRM_MS + 500, steady(ROOM), syllables);
		expect(barge.words("use the other machine", 1_050 + CONFIRM_MS + 600, REPLY)).toBeNull();
	});

	test("the reply leaking into the microphone, louder than the room, is learned as its echo and pauses nothing", () => {
		const barge = new BargeIn(0, FLOOR);
		// Residual echo at half the output, well above the plain speech threshold, following the reply's syllables.
		const leak = (at: number) => syllables(at) * 0.5;
		expect(feed(barge, 0, 4_000, leak, syllables)).toEqual([]);
		// The person over the same reply is still heard.
		expect(feed(barge, 4_050, 4_400, (at) => leak(at) + 0.2, syllables)[0]?.kind).toBe("pause");
	});

	test("the reply's own start is not taken for the person before its echo is known", () => {
		const barge = new BargeIn(0, FLOOR);
		expect(feed(barge, 0, 350, (at) => syllables(at) * 0.5, syllables)).toEqual([]);
	});

	test("words that are all the reply's own are its echo, and twice in a call stops listening over it", () => {
		const barge = new BargeIn(0, FLOOR);
		feed(barge, 0, 1_000, steady(ROOM), syllables);
		feed(barge, 1_050, 1_300, steady(0.2), syllables, "");
		expect(barge.words("the failing step is the linker", 1_800, REPLY)).toBeNull();
		const first = feed(barge, 1_850, 2_800, steady(ROOM), () => 0);
		expect(first.map((one) => one.kind)).toEqual(["resume"]);
		expect(first[0]?.why).toContain("the reply's own");
		feed(barge, 2_850, 4_400, steady(ROOM), syllables);
		feed(barge, 4_450, 4_800, steady(0.2), syllables, "the failing step is the linker");
		expect(barge.paused).toBe(true);
		expect(barge.words("the failing step is the linker which ran out of memory", 5_200, REPLY)).toBeNull();
		const second = feed(barge, 5_250, 6_000, steady(ROOM), () => 0);
		expect(second.map((one) => one.kind)).toEqual(["off"]);
		expect(barge.paused).toBe(false);
	});

	test("the reply's own words heard before the person's do not count against them", () => {
		const barge = new BargeIn(0, FLOOR);
		feed(barge, 0, 1_000, steady(ROOM), syllables);
		feed(barge, 1_050, 1_300, steady(0.2), syllables, "");
		expect(barge.words("the failing step", 1_500, REPLY)).toBeNull();
		expect(barge.words("the failing step hold on", 1_900, REPLY)?.kind).toBe("interrupt");
	});

	test("listening sounds over the reply are not an interruption, and one cut-in word is", () => {
		const barge = new BargeIn(0, FLOOR);
		feed(barge, 0, 1_000, steady(ROOM), syllables);
		feed(barge, 1_050, 1_300, steady(0.2), syllables);
		expect(barge.words("yeah mm-hmm okay", 1_800, REPLY)).toBeNull();
		expect(barge.words("yeah mm-hmm okay wait", 2_000, REPLY)?.kind).toBe("interrupt");
	});
});

describe("the person's words after an interruption", () => {
	test("the engine's words from before the onset and the reply heard back in order are not theirs", () => {
		const script = REPLY;
		expect(personWords("I looked through the wait, stop. Use the other machine.", "I looked through the", script)).toBe("wait, stop. Use the other machine.");
		// The engine re-spelt the reply's words it had before the onset.
		expect(personWords("I look through the build logs and the Wait, stop.", "I looked through the build logs and the", script)).toBe("Wait, stop.");
		// The reply's words heard after the onset, three or more in order.
		expect(personWords("the failing step is Use staging instead", "", script)).toBe("Use staging instead");
		// The person's own words may share a word or two with the reply.
		expect(personWords("The build failed again?", "", script)).toBe("The build failed again?");
	});
});
