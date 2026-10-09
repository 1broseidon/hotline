import { describe, expect, test } from "bun:test";
import type { CapabilityJob, CapabilityOptions } from "../src/generated/contract";
import { AUTOMATIC, carriedEffort, choicesFor, effortChoices, effortsOf, currentId, parseCap, pickId, spentText, splitPickId, tagsFor, usd, voiceChoices, voicePatch } from "../src/useFor";

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

	test("offers ChatGPT subscription images without selecting them automatically", () => {
		const subscription = { providerId: "openai-codex", providerName: "Codex (ChatGPT subscription)", models: [{ id: "gpt-image-2" }] };
		const job: CapabilityJob = { options: [subscription] };
		expect(choicesFor(job)).toEqual([
			{ id: AUTOMATIC, name: "Automatic" },
			{ id: "openai-codex|gpt-image-2", name: "gpt-image-2", group: "Codex (ChatGPT subscription)" },
		]);
		expect(currentId(job)).toBe(AUTOMATIC);
		expect(currentId({ ...job, selected: { providerId: subscription.providerId, providerName: subscription.providerName } })).toBe("openai-codex|gpt-image-2");
		expect(splitPickId("openai-codex|gpt-image-2")).toEqual({ providerId: "openai-codex", modelId: "gpt-image-2" });
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

	test("says what is spent today and this month; the caps are beside it", () => {
		expect(spentText(spending)).toBe("$0.14 spent today · $1.02 this month");
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

	test("count a sign-in's own speech toward that provider's connection", () => {
		const signedIn: CapabilityOptions = {
			...options,
			dispatcher: { options: [{ providerId: "xai", providerName: "xAI", models: [{ id: "grok-4.3" }] }] },
			images: { options: [{ providerId: "xai", providerName: "xAI", models: [{ id: "grok-imagine-image" }] }] },
			stt: { options: [{ providerId: "xai-subscription", providerName: "Grok subscription", models: [{ id: "grok-stt" }] }] },
			tts: { options: [{ providerId: "xai-subscription", providerName: "Grok subscription", models: [{ id: "grok-voice-tts-1.0" }] }] },
		};
		expect(tagsFor(signedIn, "xai")).toEqual(["Chat", "Images", "Voice"]);
	});
});

describe("the call assistant's thinking", () => {
	const dispatcher: CapabilityJob = {
		automatic: { providerId: "groq", providerName: "Groq", modelId: "llama-3.3-70b" },
		selected: { providerId: "anthropic", providerName: "Anthropic", modelId: "claude-sonnet-4-6", effort: "low" },
		options: [
			{ providerId: "groq", providerName: "Groq", models: [{ id: "llama-3.3-70b" }] },
			{
				providerId: "anthropic",
				providerName: "Anthropic",
				models: [
					{ id: "claude-sonnet-4-6", efforts: ["low", "medium", "high", "max"] },
					{ id: "claude-opus-5-5", efforts: ["low", "medium", "high", "xhigh", "max"] },
					{ id: "claude-haiku-4-5" },
				],
			},
		],
	};

	test("offers the picked model's levels after its own", () => {
		expect(effortChoices(dispatcher).map((one) => one.name)).toEqual(["Default thinking", "Low thinking", "Medium thinking", "High thinking", "Max thinking"]);
	});

	test("has no levels on Automatic or on a model that takes none", () => {
		expect(effortsOf({ ...dispatcher, selected: undefined })).toEqual([]);
		expect(effortsOf({ ...dispatcher, selected: { providerId: "groq", providerName: "Groq", modelId: "llama-3.3-70b" } })).toEqual([]);
	});

	test("carries the level to a model that lists it and drops it otherwise", () => {
		expect(carriedEffort(dispatcher, "anthropic", "claude-opus-5-5")).toBe("low");
		expect(carriedEffort(dispatcher, "anthropic", "claude-haiku-4-5")).toBeUndefined();
		expect(carriedEffort(dispatcher, "groq", "llama-3.3-70b")).toBeUndefined();
	});
});
