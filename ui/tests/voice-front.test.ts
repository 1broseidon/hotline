import { describe, expect, test } from "bun:test";
import type { CapabilityLiveModel } from "../src/generated/contract";
import { CALL_WORDS, OFF, TURN_BY_TURN, callKind, callModelChoices, callModelId, callModelPick, callRows, frontOf, frontPatch } from "../src/useFor";

describe("the call model setting", () => {
	test("is Grok Voice until another live model or Turn by turn is picked", () => {
		expect(frontOf(undefined)).toEqual({ provider: "grok", voices: {} });
		expect(frontOf({ front: { voices: { grok: "eve" } } })).toEqual({ provider: "grok", voices: { grok: "eve" } });
		expect(frontOf({ front: { provider: "grok", credential: "xai", voices: { grok: "ara" } } })).toEqual({ provider: "grok", credential: "xai", voices: { grok: "ara" } });
		expect(frontOf({ front: { provider: TURN_BY_TURN, voices: {} } }).provider).toBe(TURN_BY_TURN);
		expect(frontOf({ front: { provider: "gemini-live" } }).provider).toBe(TURN_BY_TURN);
		expect(frontOf({ front: { provider: "gpt-live", credential: "openai-codex", voices: { "gpt-live": "maple" } } })).toEqual({ provider: "gpt-live", credential: "openai-codex", voices: { "gpt-live": "maple" } });
		expect(frontOf({ front: { provider: "openai", credential: "openai", voices: { openai: "cedar" } } })).toEqual({ provider: "openai", credential: "openai", voices: { openai: "cedar" } });
	});

	test("a saved Legacy choice reads as Turn by turn, keeping its transcription, speech and voices", () => {
		const saved = { front: { provider: "legacy", voices: { grok: "rex" } }, stt: { provider: "local", model: "parakeet" }, tts: { provider: "ollama", model: "phonon", voice: "zoey" } };
		const front = frontOf(saved);
		expect(front).toEqual({ provider: TURN_BY_TURN, voices: { grok: "rex" } });
		expect(callKind(front)).toBe("turn-by-turn");
		expect(callModelId([], front)).toBe(TURN_BY_TURN);
		// Going live from it keeps every other part of the setting.
		expect(frontPatch(saved, { provider: "grok", credential: "xai" })).toEqual({ ...saved, front: { provider: "grok", credential: "xai", voices: { grok: "rex" } } });
	});

	test("picking a login keeps the rest of voice, and Turn by turn keeps the voice picked", () => {
		const on = frontPatch({ listenFor: ["Ophelia"] }, { provider: "grok", credential: "xai-subscription" });
		expect(on).toEqual({ listenFor: ["Ophelia"], front: { provider: "grok", credential: "xai-subscription", voices: {} } });
		const picked = frontPatch(on, { provider: "grok", voice: { provider: "grok", voice: "rex" } });
		expect(frontOf(picked).voices).toEqual({ grok: "rex" });
		const turnByTurn = frontPatch(picked, callModelPick(TURN_BY_TURN));
		expect(frontOf(turnByTurn).provider).toBe(TURN_BY_TURN);
		expect(turnByTurn).toEqual({ listenFor: ["Ophelia"], front: { provider: TURN_BY_TURN, voices: { grok: "rex" } } });
	});

	test("the default with nothing picked removes the key, and the setting with it", () => {
		expect(frontPatch({ front: { provider: TURN_BY_TURN, voices: {} } }, { provider: "grok" })).toBeNull();
	});

	test("Off is its own choice, kept apart from Turn by turn", () => {
		const off = frontPatch({ listenFor: ["Ophelia"] }, callModelPick(OFF));
		expect(off).toEqual({ listenFor: ["Ophelia"], front: { provider: OFF, voices: {} } });
		expect(frontOf(off).provider).toBe(OFF);
		expect(frontOf(frontPatch(off, { provider: "grok" })).provider).toBe("grok");
	});
});

const live = (front: string, credential: string, name: string, providerName: string): CapabilityLiveModel => ({ front, credential, modelId: `${front}-model`, name, providerName, voices: ["eve", "ara"] });

describe("the Calls model picker", () => {
	const connected = [live("grok", "xai-subscription", "Grok Voice", "Grok subscription"), live("openai", "openai", "GPT Realtime", "OpenAI")];

	test("groups the desk's live models, then Turn by turn, then Off", () => {
		const choices = callModelChoices(connected, frontOf(undefined));
		expect(choices.map((one) => [one.group, one.name, one.detail])).toEqual([
			["Live", "Grok Voice", "Grok subscription"],
			["Live", "GPT Realtime", "OpenAI"],
			["Turn by turn", "Turn by turn", "teammate's own model"],
			["No calls", "Off", "No calls"],
		]);
		expect(callModelPick(choices[0]!.id)).toEqual({ provider: "grok", credential: "xai-subscription" });
		expect(callModelId(connected, frontOf(undefined))).toBe(choices[0]!.id);
	});

	test("lists no live model when no provider that runs one is connected", () => {
		expect(callModelChoices([], frontOf({ front: { provider: TURN_BY_TURN } })).map((one) => one.group)).toEqual(["Turn by turn", "No calls"]);
	});

	test("keeps a chosen live model whose provider went away, said to be not connected", () => {
		const front = frontOf({ front: { provider: "gpt-live", credential: "openai-codex" } });
		const gone = callModelChoices(connected, front).find((one) => one.id === callModelId(connected, front));
		expect(gone).toEqual({ id: "gpt-live|openai-codex", name: "GPT-Live", detail: "Not connected", group: "Live" });
	});

	test("shows only the rows each choice has, with its own words", () => {
		expect(callRows("live")).toEqual(["Model", "Voice", "Call assistant"]);
		expect(callRows("turn-by-turn")).toEqual(["Model", "Transcription", "Speech", "Call assistant"]);
		expect(callRows("off")).toEqual(["Model"]);
		expect(CALL_WORDS.live).toEqual({ detail: "Hears, talks and uses tools in one stream", hint: "Talk over it to interrupt." });
		expect(CALL_WORDS["turn-by-turn"]).toEqual({ detail: "The teammate answers; the desk transcribes and speaks", hint: "Tap to interrupt." });
	});
});
