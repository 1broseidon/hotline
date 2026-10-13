import { describe, expect, test } from "bun:test";
import type { Look } from "../src/generated/contract";
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { faceFor } = await import("../src/faces/face");

const on = { on: true, stage: true, mark: true, doze: true };
const clip = { hash: "a".repeat(64), mimeType: "video/mp4" };
const look = (style: Look["style"], clips: Look["clips"] = {}): Look => ({ style, clips, updatedAt: "2026-10-10T00:00:00.000Z" });

describe("a teammate's face", () => {
	test("is drawn as it always was while the window draws no faces, or the teammate has no look", () => {
		expect(faceFor({ ...on, on: false }, look("video", { idle: clip }))).toBeNull();
		expect(faceFor(on, undefined)).toBeNull();
		expect(faceFor(on, look("photo"))).toBeNull();
	});

	test("is the initial when the person chose it, even over a picture", () => {
		expect(faceFor(on, look("initial"))).toEqual({ kind: "initial" });
	});

	test("is video once it has its idle clip, and the picture until then", () => {
		expect(faceFor(on, look("video", { idle: clip, done: clip }))?.kind).toBe("video");
		expect(faceFor(on, look("video", { done: clip }))).toBeNull();
	});
});
