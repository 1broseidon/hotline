import { describe, expect, test } from "bun:test";
import type { CapabilityOptions, SpeechModel } from "../src/generated/contract";
import { busy, installedChanged, installedSummary, languageHint, megabytes, modelKind, modelLine, progress, usedForCalls } from "../src/voice/deskModels";

function model(overrides: Partial<SpeechModel> = {}): SpeechModel {
	return {
		id: "parakeet-tdt-110m-en",
		name: "Parakeet English",
		detail: "English only, small and quick",
		downloadBytes: 108_035_095,
		diskBytes: 136_490_421,
		credit: "NVIDIA Parakeet TDT 110M, CC BY 4.0, quantized by sherpa-onnx",
		licenceUrl: "https://creativecommons.org/licenses/by/4.0/",
		state: "available",
		...overrides,
	};
}

describe("the desk's speech models in words", () => {
	test("sizes read as a download dialog says them", () => {
		expect(megabytes(108_035_095)).toBe("108 MB");
		expect(megabytes(487_170_055)).toBe("487 MB");
		expect(megabytes(1_200_000_000)).toBe("1.2 GB");
		expect(megabytes(10)).toBe("1 MB");
	});

	test("each state says where the model stands", () => {
		expect(modelLine(model())).toBe("");
		expect(modelLine(model({ state: "downloading", receivedBytes: 54_000_000 }))).toBe("Downloading 54 MB of 108 MB");
		expect(progress(model({ state: "downloading", receivedBytes: 54_017_547 }))).toBeCloseTo(0.5, 3);
		expect(modelLine(model({ state: "unpacking" }))).toBe("Unpacking");
		expect(modelLine(model({ state: "installed" }))).toBe("136 MB");
		// A model the catalogue no longer lists has nothing more to say.
		expect(modelLine(model({ state: "installed", detail: "", diskBytes: 0 }))).toBe("");
	});

	test("only a download under way is asked after, and only a change in what is installed refreshes hearing", () => {
		expect(busy([model(), model({ state: "installed" })])).toBe(false);
		expect(busy([model(), model({ state: "downloading" })])).toBe(true);
		expect(busy([model({ state: "unpacking" })])).toBe(true);
		const before = [model({ state: "downloading" })];
		expect(installedChanged(before, [model({ state: "unpacking" })])).toBe(false);
		expect(installedChanged(before, [model({ state: "installed" })])).toBe(true);
		expect(installedChanged([model({ state: "installed" })], [model()])).toBe(true);
	});
});

describe("which model to suggest", () => {
	const parakeet = model({ id: "parakeet-tdt-0.6b-v3", name: "Parakeet", tag: "25 languages" });
	const english = model({ state: "installed", tag: "fast", moreLanguages: "parakeet-tdt-0.6b-v3" });

	test("the bigger Parakeet is suggested only to a window not in English, while the English one is all there is", () => {
		expect(languageHint(english, [parakeet, english], "fr-FR")).toBe("Parakeet hears more languages");
		// In English the small model hears as well, and faster: nothing to suggest.
		expect(languageHint(english, [parakeet, english], "en-GB")).toBeNull();
		expect(languageHint(english, [parakeet, english], "EN")).toBeNull();
		// Not once the other is coming or there, nor while another model hears.
		expect(languageHint(english, [{ ...parakeet, state: "downloading" }, english], "de")).toBeNull();
		expect(languageHint(english, [{ ...parakeet, state: "installed" }, english], "de")).toBeNull();
		expect(languageHint(english, [parakeet, english, model({ id: "whisper-large-v3-turbo", name: "Whisper", state: "installed" })], "de")).toBeNull();
		// Not on a row that is not installed, nor one that names no other.
		expect(languageHint({ ...english, state: "available" }, [parakeet, english], "de")).toBeNull();
		expect(languageHint(parakeet, [parakeet, english], "de")).toBeNull();
	});
});

describe("the Voice models row and page", () => {
	const options = (overrides: Partial<CapabilityOptions> = {}): CapabilityOptions => ({
		images: { options: [] },
		stt: { options: [] },
		tts: { options: [] },
		dispatcher: { options: [] },
		spending: { budgets: [] },
		...overrides,
	});
	const desk = (models: { id: string; label: string }[]) => ({ providerId: "local", providerName: "On the desk", models });

	test("the row sums what is installed, and says when nothing is", () => {
		expect(installedSummary([model()])).toBe("None installed");
		expect(installedSummary([model({ state: "installed", diskBytes: 670_000_000 }), model({ id: "phonon-31m-en", state: "installed", diskBytes: 66_000_000 }), model({ id: "w" })])).toBe("2 installed · 736 MB");
		// A model the catalogue no longer lists has no size to add.
		expect(installedSummary([model({ state: "installed", diskBytes: 0 })])).toBe("1 installed");
	});

	test("the desk's voice speaks before it is installed, a model Speech lists speaks, and every other one transcribes", () => {
		expect(modelKind(model({ id: "phonon-31m-en", name: "Phonon" }), null)).toBe("Speech");
		expect(modelKind(model(), null)).toBe("Transcription");
		const speaking = options({ tts: { options: [desk([{ id: "another-voice", label: "Another" }])] } });
		expect(modelKind(model({ id: "another-voice", state: "installed" }), speaking)).toBe("Speech");
		// A desk model Transcription lists is not a speaker for that.
		expect(modelKind(model({ state: "installed" }), options({ stt: { options: [desk([{ id: "parakeet-tdt-110m-en", label: "Parakeet English" }])] } }))).toBe("Transcription");
	});

	test("a model is used for calls while Turn by turn's Transcription or Speech runs on it, chosen or by Automatic", () => {
		const parakeet = model({ state: "installed" });
		const phonon = model({ id: "phonon-31m-en", name: "Phonon", state: "installed" });
		const stt = { automatic: { providerId: "local", providerName: "On the desk", modelId: "parakeet-tdt-110m-en" }, options: [desk([{ id: "parakeet-tdt-110m-en", label: "Parakeet English" }])] };
		const tts = { selected: { providerId: "local", providerName: "On the desk", modelId: "phonon-31m-en" }, options: [desk([{ id: "phonon-31m-en", label: "Phonon" }])] };
		const turnByTurn = { front: { provider: "turn-by-turn" } };
		expect(usedForCalls(parakeet, options({ stt, tts }), turnByTurn)).toBe(true);
		expect(usedForCalls(phonon, options({ stt, tts }), turnByTurn)).toBe(true);
		// Not while another provider hears, nor while a live model runs calls itself, nor with calls off.
		expect(usedForCalls(parakeet, options({ stt: { options: stt.options, automatic: { providerId: "openai", providerName: "OpenAI", modelId: "gpt-4o-transcribe" } }, tts }), turnByTurn)).toBe(false);
		expect(usedForCalls(parakeet, options({ stt, tts }), { front: { provider: "grok" } })).toBe(false);
		expect(usedForCalls(phonon, options({ stt, tts }), { front: { provider: "off" } })).toBe(false);
	});
});
