import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { moodPicture } from "../src/faces/drawing";
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

	test("is drawn once it has its drawing, and the picture until then", () => {
		const drawing = { hash: "b".repeat(64), mimeType: "image/svg+xml" };
		expect(faceFor(on, { ...look("drawn"), drawing })).toEqual({ kind: "drawn", drawing });
		expect(faceFor(on, look("drawn"))).toBeNull();
	});
});

describe("a drawing's picture of a mood", () => {
	const page = new Window();
	const parser = new page.DOMParser() as unknown as DOMParser;
	const serializer = new page.XMLSerializer() as unknown as XMLSerializer;
	const drawing = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><g data-part="head"/></svg>';

	test("carries the rig and names its mood on the root", () => {
		const picture = moodPicture(drawing, "needy", "[data-mood] {}", parser, serializer) ?? "";
		expect(picture).toContain('data-mood="needy"');
		expect(picture).toContain("[data-mood] {}");
		expect(picture).toContain('data-part="head"');
	});

	test("is nothing for what is not an SVG", () => {
		expect(moodPicture("<html><body/></html>", "idle", "", parser, serializer)).toBeNull();
		expect(moodPicture("<svg", "idle", "", parser, serializer)).toBeNull();
	});
});
