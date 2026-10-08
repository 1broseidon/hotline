import { describe, expect, test } from "bun:test";
import { toadPose } from "../src/components/CallToad";
import { speechLevel } from "../src/voice/call";

describe("the reply's level", () => {
	test("is silent at silence and full at a loud reply, on the phone's curve", () => {
		expect(speechLevel(0)).toBe(0);
		expect(speechLevel(0.1)).toBeCloseTo(Math.pow(0.5, 0.65), 6);
		expect(speechLevel(0.2)).toBe(1);
		expect(speechLevel(0.9)).toBe(1);
	});

	test("rises with the audio, so louder syllables open the mouth further", () => {
		expect(speechLevel(0.02)).toBeLessThan(speechLevel(0.05));
		expect(speechLevel(0.05)).toBeLessThan(speechLevel(0.1));
	});
});

describe("the call toad's pose", () => {
	// Mid-beat: the eyes are open.
	const open = 1;

	test("the mouth is a slit unless the desk speaks, and the reply opens it", () => {
		for (const phase of ["connecting", "listening", "hearing", "thinking", "held", "ended"] as const) {
			const pose = toadPose(phase, 1, open, false);
			expect(pose.mouthWidth).toBeCloseTo(0.7);
			expect(pose.mouthHeight).toBeCloseTo(0.15);
		}
		const quiet = toadPose("speaking", 0, open, false);
		const loud = toadPose("speaking", 1, open, false);
		expect(quiet.mouthHeight).toBeCloseTo(0.15);
		expect(loud.mouthWidth).toBeCloseTo(1);
		expect(loud.mouthHeight).toBeCloseTo(1.7);
		expect(toadPose("speaking", 0.5, open, false).mouthHeight).toBeCloseTo(0.925);
	});

	test("speaking lifts the toad with the reply; thinking looks up and bobs", () => {
		expect(toadPose("speaking", 0, open, false).nod).toBeCloseTo(0);
		expect(toadPose("speaking", 1, open, false).nod).toBeCloseTo(-1.8 * 56 / 92);
		expect(toadPose("thinking", 0, open, false).pupils).toBe(-1.5);
		expect(toadPose("listening", 0, open, false).pupils).toBe(0);
		const bob = [0, 0.5, 1, 1.5, 2].map((seconds) => toadPose("thinking", 0, seconds, false).nod);
		expect(new Set(bob.map((nod) => nod.toFixed(3))).size).toBeGreaterThan(1);
		expect(Math.max(...bob.map(Math.abs))).toBeLessThanOrEqual(1.5 * 56 / 92 + 1e-9);
	});

	test("it blinks once every 4.1 seconds, and hold droops the eyes", () => {
		expect(toadPose("listening", 0, open, false).eyes).toBe(1);
		expect(toadPose("listening", 0, 3.975, false).eyes).toBeCloseTo(0.12);
		expect(toadPose("listening", 0, 4.1 + 3.975, false).eyes).toBeCloseTo(0.12);
		expect(toadPose("held", 0, open, false).eyes).toBeCloseTo(0.75);
	});

	test("reduced motion holds it still: no blink, no bob, the mouth half open while it speaks", () => {
		for (const seconds of [0, 1, 3.975, 7]) {
			expect(toadPose("listening", 0, seconds, true).eyes).toBe(1);
			expect(toadPose("thinking", 0, seconds, true).nod).toBe(0);
			const speaking = toadPose("speaking", 0.8, seconds, true);
			expect(speaking.nod).toBe(0);
			expect(speaking.mouthWidth).toBeCloseTo(0.79);
			expect(speaking.mouthHeight).toBeCloseTo(0.615);
		}
		expect(toadPose("thinking", 0, 0, true).pupils).toBe(-1.5);
		expect(toadPose("held", 0, 0, true).eyes).toBeCloseTo(0.75);
	});
});
