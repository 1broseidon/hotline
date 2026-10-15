import { describe, expect, test } from "bun:test";
import { frontOf, frontPatch } from "../src/useFor";

describe("the voice front setting", () => {
	test("is Off until a provider is picked", () => {
		expect(frontOf(undefined)).toBeUndefined();
		expect(frontOf({ front: { voices: { grok: "eve" } } })).toBeUndefined();
		expect(frontOf({ front: { provider: "grok", credential: "xai", voices: { grok: "ara" } } })).toEqual({ provider: "grok", credential: "xai", voices: { grok: "ara" } });
	});

	test("turning it on keeps the rest of voice, and Off keeps the voice picked", () => {
		const on = frontPatch({ listenFor: ["Ophelia"] }, { provider: "grok", credential: "xai-subscription" });
		expect(on).toEqual({ listenFor: ["Ophelia"], front: { provider: "grok", credential: "xai-subscription", voices: {} } });
		const picked = frontPatch(on, { provider: "grok", voice: { provider: "grok", voice: "rex" } });
		expect(frontOf(picked)?.voices).toEqual({ grok: "rex" });
		const off = frontPatch(picked, { provider: null });
		expect(frontOf(off)).toBeUndefined();
		expect(off).toEqual({ listenFor: ["Ophelia"], front: { voices: { grok: "rex" } } });
	});

	test("Off with nothing picked removes the key, and the setting with it", () => {
		expect(frontPatch({ front: { provider: "grok", voices: {} } }, { provider: null })).toBeNull();
	});
});
