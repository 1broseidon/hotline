import { describe, expect, test } from "bun:test";
import type { CapabilityJob, CapabilityOptions } from "../src/generated/contract";
import { AUTOMATIC, choicesFor, currentId, parseCap, pickId, spentText, splitPickId, tagsFor, usd, voiceChoices, voicePatch } from "../src/useFor";

const images: CapabilityJob = {
	automatic: { providerId: "openrouter", providerName: "OpenRouter", modelId: "openai/gpt-image-2.5-flare" },
	options: [
		{ providerId: "openrouter", providerName: "OpenRouter", models: [{ id: "openai/gpt-image-2.5-flare" }, { id: "google/gemini-3.1-flash-image" }] },
		{ providerId: "openai", providerName: "OpenAI", models: [{ id: "gpt-image-2.5-flare" }] },
	],
};

describe("a Use for picker", () => {
	test("starts with Automatic, named for what it comes to now", () => {
		const choices = choicesFor(images);
		expect(choices[0]).toEqual({ id: AUTOMATIC, name: "Automatic · OpenRouter · gpt-image-2.5-flare" });
		expect(choices.slice(1).map((one) => [one.group, one.name])).toEqual([
			["OpenRouter", "gpt-image-2.5-flare"],
			["OpenRouter", "gemini-3.1-flash-image"],
			["OpenAI", "gpt-image-2.5-flare"],
		]);
	});

	test("is just Automatic when nothing resolves", () => {
		expect(choicesFor({ options: [] })).toEqual([{ id: AUTOMATIC, name: "Automatic" }]);
	});

	test("keeps a choice whose provider was disconnected, under its own heading", () => {
		const job = { ...images, selected: { providerId: "google", providerName: "google", modelId: "gemini-3-pro-image" } };
		const gone = choicesFor(job).at(-1);
		expect(gone).toEqual({ id: pickId("google", "gemini-3-pro-image"), name: "google · gemini-3-pro-image", group: "Not connected" });
		expect(currentId(job)).toBe(gone?.id);
	});

	test("reads a provider named alone as its first model", () => {
		expect(currentId({ ...images, selected: { providerId: "openai", providerName: "OpenAI" } })).toBe(pickId("openai", "gpt-image-2.5-flare"));
		expect(currentId(images)).toBe(AUTOMATIC);
	});

	test("round-trips an id whose model has slashes", () => {
		expect(splitPickId(pickId("openrouter", "x-ai/grok-imagine-image-2.0"))).toEqual({ providerId: "openrouter", modelId: "x-ai/grok-imagine-image-2.0" });
	});
});

describe("the speaking row's voices", () => {
	const tts: CapabilityJob = {
		automatic: { providerId: "openai", providerName: "OpenAI", modelId: "gpt-4o-mini-tts", voice: "marin" },
		options: [{ providerId: "openai", providerName: "OpenAI", models: [{ id: "gpt-4o-mini-tts", voices: ["marin", "cedar"] }] }],
	};

	test("are the model's, led by its default", () => {
		expect(voiceChoices(tts).map((one) => one.name)).toEqual(["Default · marin", "marin", "cedar"]);
	});

	test("keep a voice the owner named that the list no longer has", () => {
		const job = { ...tts, selected: { providerId: "openai", providerName: "OpenAI", modelId: "gpt-4o-mini-tts", voice: "verse" } };
		expect(voiceChoices(job).at(-1)).toEqual({ id: "verse", name: "verse" });
	});
});

describe("a write to settings.voice", () => {
	test("changes one job and keeps the caps, the fallback and the other jobs", () => {
		const current = { dayUsd: 5, fallbackTts: { provider: "google" }, stt: { provider: "groq" } };
		expect(voicePatch(current, "tts", { provider: "openai", model: "gpt-4o-mini-tts", voice: "cedar" })).toEqual({
			dayUsd: 5,
			fallbackTts: { provider: "google" },
			stt: { provider: "groq" },
			tts: { provider: "openai", model: "gpt-4o-mini-tts", voice: "cedar" },
		});
	});

	test("Automatic removes the key, and an empty voice setting is cleared", () => {
		expect(voicePatch({ stt: { provider: "groq" }, dayUsd: 5 }, "stt", null)).toEqual({ dayUsd: 5 });
		expect(voicePatch({ stt: { provider: "groq" } }, "stt", null)).toBeNull();
		expect(voicePatch(undefined, "dispatcher", null)).toBeNull();
	});
});

describe("the spending line", () => {
	const spending = { dayUsd: 2, monthUsd: 20, spentDayUsd: 0.14, spentMonthUsd: 1.02 };

	test("says what is spent against each cap", () => {
		expect(spentText(spending)).toBe("$0.14 of $2.00 today · $1.02 of $20.00 this month");
	});

	test("never shows a real spend as nothing, and owns up to an unread tally", () => {
		expect(usd(0.002)).toBe("<$0.01");
		expect(usd(0)).toBe("$0.00");
		expect(spentText({ ...spending, unavailable: "x" })).toContain("could not be read");
	});

	test("reads a cap as a non-negative dollar amount", () => {
		expect(parseCap("$1.5")).toBe(1.5);
		expect(parseCap("0")).toBe(0);
		expect(parseCap("-1")).toBeNull();
		expect(parseCap("abc")).toBeNull();
		expect(parseCap("")).toBeNull();
	});
});

describe("a connection's tags", () => {
	const job = (...ids: string[]): CapabilityJob => ({ options: ids.map((providerId) => ({ providerId, providerName: providerId, models: [] })) });
	const options: CapabilityOptions = {
		images: job("openai"),
		stt: job("openai", "groq"),
		tts: job("openai"),
		dispatcher: job("openai", "anthropic"),
		spending: { dayUsd: 2, monthUsd: 20, spentDayUsd: 0, spentMonthUsd: 0 },
	};

	test("name what the provider can be used for", () => {
		expect(tagsFor(options, "openai")).toEqual(["Chat", "Images", "Voice"]);
		expect(tagsFor(options, "groq")).toEqual(["Voice"]);
		expect(tagsFor(options, "anthropic")).toEqual(["Chat"]);
		expect(tagsFor(options, "mistral")).toEqual([]);
	});
});
