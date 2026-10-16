import { describe, expect, test } from "bun:test";
import { LEGACY, OFF, frontOf, frontPatch } from "../src/useFor";

describe("the voice front setting", () => {
	test("is Grok Voice until Legacy voice is picked", () => {
		expect(frontOf(undefined)).toEqual({ provider: "grok", voices: {} });
		expect(frontOf({ front: { voices: { grok: "eve" } } })).toEqual({ provider: "grok", voices: { grok: "eve" } });
		expect(frontOf({ front: { provider: "grok", credential: "xai", voices: { grok: "ara" } } })).toEqual({ provider: "grok", credential: "xai", voices: { grok: "ara" } });
		expect(frontOf({ front: { provider: LEGACY, voices: {} } }).provider).toBe(LEGACY);
		expect(frontOf({ front: { provider: "gemini-live" } }).provider).toBe(LEGACY);
		expect(frontOf({ front: { provider: "gpt-live", credential: "openai-codex", voices: { "gpt-live": "maple" } } })).toEqual({ provider: "gpt-live", credential: "openai-codex", voices: { "gpt-live": "maple" } });
		expect(frontOf({ front: { provider: "openai", credential: "openai", voices: { openai: "cedar" } } })).toEqual({ provider: "openai", credential: "openai", voices: { openai: "cedar" } });
	});

	test("picking a login keeps the rest of voice, and Legacy voice keeps the voice picked", () => {
		const on = frontPatch({ listenFor: ["Ophelia"] }, { provider: "grok", credential: "xai-subscription" });
		expect(on).toEqual({ listenFor: ["Ophelia"], front: { provider: "grok", credential: "xai-subscription", voices: {} } });
		const picked = frontPatch(on, { provider: "grok", voice: { provider: "grok", voice: "rex" } });
		expect(frontOf(picked).voices).toEqual({ grok: "rex" });
		const legacy = frontPatch(picked, { provider: LEGACY });
		expect(frontOf(legacy).provider).toBe(LEGACY);
		expect(legacy).toEqual({ listenFor: ["Ophelia"], front: { provider: LEGACY, voices: { grok: "rex" } } });
	});

	test("the default with nothing picked removes the key, and the setting with it", () => {
		expect(frontPatch({ front: { provider: LEGACY, voices: {} } }, { provider: "grok" })).toBeNull();
	});

	test("Off is its own choice, kept apart from Legacy voice", () => {
		const off = frontPatch({ listenFor: ["Ophelia"] }, { provider: OFF });
		expect(off).toEqual({ listenFor: ["Ophelia"], front: { provider: OFF, voices: {} } });
		expect(frontOf(off).provider).toBe(OFF);
		expect(frontOf(frontPatch(off, { provider: "grok" })).provider).toBe("grok");
	});
});
