import type { CapabilityOptions, SpeechModel } from "../generated/contract";
import { callFront, callKind, deskModelOf } from "../useFor";

/** The desk's own models, as the core names their provider. */
const ON_THE_DESK = "local";

/**
 * The desk's own speech models as Settings › Voice › Voice models shows
 * them: what each is, how big, and where its download stands. The desk
 * does the work (voice.md, Hearing on the desk); this only words it.
 */

/** A size the way a download dialog says it: megabytes, decimal, whole. */
export function megabytes(bytes: number): string {
	if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
	return `${Math.max(1, Math.round(bytes / 1e6))} MB`;
}

/** Whether something is still moving, so the window should ask again soon. */
export function busy(models: readonly SpeechModel[]): boolean {
	return models.some((model) => model.state === "downloading" || model.state === "unpacking");
}

/** How far a download has come, 0 to 1. */
export function progress(model: SpeechModel): number {
	if (model.downloadBytes <= 0) return 0;
	return Math.min(1, Math.max(0, (model.receivedBytes ?? 0) / model.downloadBytes));
}

/** Where a model stands, for its row's second line: how big once installed, how far while it comes. */
export function modelLine(model: SpeechModel): string {
	switch (model.state) {
		case "downloading":
			return `Downloading ${megabytes(model.receivedBytes ?? 0)} of ${megabytes(model.downloadBytes)}`;
		case "unpacking":
			return "Unpacking";
		case "installed":
			return model.diskBytes > 0 ? megabytes(model.diskBytes) : "";
		case "available":
			return "";
	}
}

/**
 * The one line suggesting a model that hears more languages, for a window
 * not in English, while `model` is the only one installed and the other is
 * there to download. For English the small model hears as well, and faster.
 */
export function languageHint(model: SpeechModel, models: readonly SpeechModel[], language: string): string | null {
	if (model.state !== "installed" || model.moreLanguages === undefined) return null;
	if (language.toLowerCase().startsWith("en")) return null;
	if (models.some((other) => other.id !== model.id && other.state === "installed")) return null;
	const other = models.find((one) => one.id === model.moreLanguages && one.state === "available");
	return other === undefined ? null : `${other.name} hears more languages`;
}

/** Whether the set of installed models differs, which changes what can hear. */
export function installedChanged(before: readonly SpeechModel[], after: readonly SpeechModel[]): boolean {
	const ids = (models: readonly SpeechModel[]) =>
		models
			.filter((model) => model.state === "installed")
			.map((model) => model.id)
			.sort()
			.join("\n");
	return ids(before) !== ids(after);
}

/**
 * The desk's own voice (speech/local/voice.rs `MODEL_ID`). The wire does
 * not say whether a desk model hears or speaks; once installed, Speech
 * lists it, but before that only its id tells.
 */
const DESK_VOICE = "phonon-31m-en";

/** What a desk model does, as its tag on the Voice models page. */
export type ModelKind = "Transcription" | "Speech";

/** Whether the model speaks: the desk's voice, or any desk model Speech lists. */
export function modelKind(model: SpeechModel, options: CapabilityOptions | null): ModelKind {
	const listed = options?.tts.options.some((provider) => provider.providerId === ON_THE_DESK && provider.models.some((one) => one.id === model.id));
	return model.id === DESK_VOICE || listed === true ? "Speech" : "Transcription";
}

/** What the installed models take on the desk's disk, in bytes. */
export function usedBytes(models: readonly SpeechModel[]): number {
	return models.filter((model) => model.state === "installed").reduce((sum, model) => sum + model.diskBytes, 0);
}

/** The Voice models row's quiet summary: "2 installed · 736 MB", or that none is. */
export function installedSummary(models: readonly SpeechModel[]): string {
	const count = models.filter((model) => model.state === "installed").length;
	if (count === 0) return "None installed";
	const bytes = usedBytes(models);
	return bytes > 0 ? `${count} installed · ${megabytes(bytes)}` : `${count} installed`;
}

/**
 * Whether a call uses this model: Turn by turn's Transcription or Speech
 * runs on it, as the desk's settings say (a window hearing with its own
 * Mac does not change what the phone's calls use).
 */
export function usedForCalls(model: SpeechModel, options: CapabilityOptions, voice: unknown): boolean {
	if (callKind(callFront(voice, options.live ?? [])) !== "turn-by-turn") return false;
	return deskModelOf(options.stt) === model.id || deskModelOf(options.tts) === model.id;
}
