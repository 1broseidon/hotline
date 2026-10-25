import type { CapabilityJob, CapabilityModel, CapabilityOptions, CapabilityPick, CapabilityProvider } from "./generated/contract";

/**
 * What Settings › Providers › Use for needs from `capabilities.options`:
 * the picker's choices, the writes a pick makes, and the words for the
 * spending line. Kept apart from the component so the rules can be tested
 * without a window.
 */

/** A picker's id for "no choice": the setting key is absent, so the desk picks. */
export const AUTOMATIC = "";

/** Provider ids never contain a bar, so the first one splits an id in two. */
const SEPARATOR = "|";

export type PickerChoice = { id: string; name: string; detail?: string; group?: string };

export function pickId(providerId: string, modelId: string | undefined): string {
	return `${providerId}${SEPARATOR}${modelId ?? ""}`;
}

export function splitPickId(id: string): { providerId: string; modelId: string | undefined } {
	const at = id.indexOf(SEPARATOR);
	const modelId = id.slice(at + 1);
	return { providerId: id.slice(0, at), modelId: modelId === "" ? undefined : modelId };
}

/** A model as a person reads it: a gateway's `vendor/` prefix is noise under the gateway's own heading. */
export function shortModel(id: string): string {
	return id.slice(id.lastIndexOf("/") + 1);
}

function modelText(pick: CapabilityPick): string {
	return pick.modelId === undefined ? "" : shortModel(pick.modelId);
}

/** The first choice: Automatic, and what it comes to now. */
function automaticText(job: CapabilityJob): string {
	const automatic = job.automatic;
	if (automatic === undefined) return "Automatic";
	return ["Automatic", automatic.providerName, modelText(automatic)].filter((part) => part !== "").join(" · ");
}

/**
 * Automatic, then every connected provider's models under its name. A
 * choice the owner made that is no longer on offer, because its provider was
 * disconnected, stays listed under its own heading so the picker never claims
 * the job is on something it is not.
 */
export function choicesFor(job: CapabilityJob): PickerChoice[] {
	const choices: PickerChoice[] = [{ id: AUTOMATIC, name: automaticText(job) }];
	for (const provider of job.options) {
		for (const model of provider.models) {
			choices.push({ id: pickId(provider.providerId, model.id), name: model.label ?? shortModel(model.id), group: provider.providerName });
		}
	}
	const selected = job.selected;
	if (selected !== undefined && !choices.some((one) => one.id === selectedId(selected))) {
		choices.push({
			id: selectedId(selected),
			name: [selected.providerName, modelText(selected)].filter((part) => part !== "").join(" · "),
			group: "Not connected",
		});
	}
	return choices;
}

function selectedId(pick: CapabilityPick | undefined): string {
	return pick === undefined ? AUTOMATIC : pickId(pick.providerId, pick.modelId);
}

/**
 * A provider alone, named without a model, stands for that provider's first
 * model, which is what the desk would use for it.
 */
export function currentId(job: CapabilityJob): string {
	const selected = job.selected;
	if (selected === undefined) return AUTOMATIC;
	if (selected.modelId !== undefined) return selectedId(selected);
	const first = job.options.find((one) => one.providerId === selected.providerId)?.models[0];
	return pickId(selected.providerId, first?.id);
}

/** What Ollama calls the voice a speech model speaks in when asked for none. */
const OWN_VOICE = "default";

/**
 * A voice as a person reads it: "eve" is Eve. A voice named `default` is the
 * model's own, so it reads as the model's name ("Phonon"), never "Default"
 * beside the default.
 */
export function voiceLabel(voice: string, model?: string): string {
	const name = voice === OWN_VOICE && model !== undefined ? model : voice;
	return name.charAt(0).toUpperCase() + name.slice(1);
}

/** A model's name in a picker: its label, else its id without a gateway's prefix. */
function modelName(model: CapabilityModel): string {
	return model.label ?? shortModel(model.id);
}

const VOICE_SEPARATOR = "\u001f";

function voiceId(providerId: string, modelId: string, voice: string): string {
	return [providerId, modelId, voice].join(VOICE_SEPARATOR);
}

export function splitVoiceId(id: string): { providerId: string; modelId: string; voiceId: string } {
	const [providerId = "", modelId = "", voiceId = ""] = id.split(VOICE_SEPARATOR);
	return { providerId, modelId, voiceId };
}

/** The model a pick names, or its provider's first when it names none. */
function pickedModel(job: CapabilityJob, pick: CapabilityPick): CapabilityModel | undefined {
	const provider = job.options.find((one) => one.providerId === pick.providerId);
	return provider?.models.find((one) => one.id === pick.modelId) ?? provider?.models[0];
}

/**
 * Every voice of every connected speaking model, under its provider, and
 * under the model too where a provider has several: "OpenRouter · Grok Voice TTS 1.0".
 */
export function allVoices(job: CapabilityJob): PickerChoice[] {
	const choices: PickerChoice[] = [{ id: AUTOMATIC, name: "Automatic" }];
	for (const provider of job.options) {
		for (const model of provider.models) {
			const group = provider.models.length > 1 ? `${provider.providerName} · ${modelName(model)}` : provider.providerName;
			for (const voice of model.voices ?? []) {
				choices.push({ id: voiceId(provider.providerId, model.id, voice), name: voiceLabel(voice, modelName(model)), group });
			}
		}
	}
	return choices;
}

export function currentVoiceId(job: CapabilityJob): string {
	const selected = job.selected;
	if (selected === undefined) return AUTOMATIC;
	const model = pickedModel(job, selected);
	const voice = selected.voice ?? model?.voices?.[0];
	return model === undefined || voice === undefined ? AUTOMATIC : voiceId(selected.providerId, model.id, voice);
}

/** The speaking row's second line: the voice in use and who speaks it. */
export function speakingText(job: CapabilityJob): string {
	const pick = job.selected ?? job.automatic;
	if (pick === undefined) return "Talk to the desk";
	const model = pickedModel(job, pick);
	const voice = pick.voice === undefined ? "Default voice" : voiceLabel(pick.voice, model === undefined ? undefined : modelName(model));
	return `${voice} on ${pick.providerName}`;
}

/** The thinking levels of the model the owner picked; none for Automatic, which stores no level. */
export function effortsOf(job: CapabilityJob): string[] {
	const selected = job.selected;
	if (selected === undefined) return [];
	const provider = job.options.find((one) => one.providerId === selected.providerId);
	const model = provider?.models.find((one) => one.id === selected.modelId) ?? provider?.models[0];
	return model?.efforts ?? [];
}

const EFFORT_LABELS: Record<string, string> = {
	none: "None",
	minimal: "Minimal",
	low: "Low",
	medium: "Medium",
	high: "High",
	xhigh: "Extra high",
	max: "Max",
};

function effortLabel(id: string): string {
	return EFFORT_LABELS[id] ?? id;
}

/** The model's own level first, then each one it lists. */
export function effortChoices(job: CapabilityJob): PickerChoice[] {
	return [{ id: AUTOMATIC, name: "Default thinking" }, ...effortsOf(job).map((id) => ({ id, name: `${effortLabel(id)} thinking` }))];
}

/** A choice the desk stores: `{provider, model?, voice?, effort?}`. */
export type Stored = { provider: string; model?: string; voice?: string; effort?: string };

export function stored(providerId: string, modelId: string | undefined, voice?: string, effort?: string): Stored {
	return {
		provider: providerId,
		...(modelId !== undefined ? { model: modelId } : {}),
		...(voice !== undefined ? { voice } : {}),
		...(effort !== undefined ? { effort } : {}),
	};
}

/**
 * The call assistant moved to another model: the level comes along when the
 * new model lists it, so going from one fast model to another stays "Low".
 */
export function carriedEffort(job: CapabilityJob, providerId: string, modelId: string | undefined): string | undefined {
	const effort = job.selected?.effort;
	if (effort === undefined) return undefined;
	const provider = job.options.find((one) => one.providerId === providerId);
	const model = provider?.models.find((one) => one.id === modelId) ?? provider?.models[0];
	return model?.efforts?.includes(effort) ? effort : undefined;
}

/**
 * `settings.voice` with one job changed. A write replaces the whole object,
 * so the caps, the fallback and every other job are carried over untouched;
 * `null` for a job is Automatic, which is the key's absence.
 */
export function voicePatch(current: unknown, job: "stt" | "tts" | "dispatcher", choice: Stored | null): Record<string, unknown> | null {
	const voice: Record<string, unknown> =
		current !== null && typeof current === "object" && !Array.isArray(current) ? { ...(current as Record<string, unknown>) } : {};
	if (choice === null) delete voice[job];
	else voice[job] = choice;
	return Object.keys(voice).length === 0 ? null : voice;
}

/** The words to listen for as typed: split at commas and new lines, trimmed, each once. */
export function parseWords(text: string): string[] {
	const words: string[] = [];
	for (const part of text.split(/[,\n]/)) {
		const word = part.trim();
		if (word !== "" && !words.some((one) => one.toLowerCase() === word.toLowerCase())) words.push(word);
	}
	return words;
}

/** The words `settings.voice` says to listen for (`listenFor`). */
export function listenForOf(voice: unknown): string[] {
	if (voice === null || typeof voice !== "object" || Array.isArray(voice)) return [];
	const words = (voice as Record<string, unknown>)["listenFor"];
	return Array.isArray(words) ? words.filter((word): word is string => typeof word === "string") : [];
}

/** `settings.voice` with its words to listen for replaced, keeping the rest; none takes the key away. */
export function listenForPatch(current: unknown, words: string[]): Record<string, unknown> | null {
	const voice: Record<string, unknown> =
		current !== null && typeof current === "object" && !Array.isArray(current) ? { ...(current as Record<string, unknown>) } : {};
	if (words.length === 0) delete voice["listenFor"];
	else voice["listenFor"] = words;
	return Object.keys(voice).length === 0 ? null : voice;
}

/**
 * Speech a sign-in pays for is listed under an id of its own, so voice can
 * say which it is using, but it comes from that provider's one connection.
 */
const SPEECH_FROM: Record<string, string> = { "xai-subscription": "xai" };

/** Whether a provider offers anything in any job, for the tags on its connection. */
export function tagsFor(options: CapabilityOptions, providerId: string): string[] {
	const offers = (providers: CapabilityProvider[]) =>
		providers.some((one) => (SPEECH_FROM[one.providerId] ?? one.providerId) === providerId);
	const tags: string[] = [];
	if (offers(options.dispatcher.options)) tags.push("Chat");
	if (offers(options.images.options)) tags.push("Images");
	// A login that runs a voice front does voice even with no speech job (the ChatGPT sign-in).
	if (offers(options.stt.options) || offers(options.tts.options) || FRONT_LOGINS.includes(providerId)) tags.push("Voice");
	return tags;
}

/** Dollars as a person says them; a spend too small to show is not zero. */
export function usd(amount: number): string {
	if (amount > 0 && amount < 0.005) return "<$0.01";
	return `$${amount.toFixed(2)}`;
}

/** A cap typed into a field: a finite, non-negative dollar amount, else nothing. */
export function parseCap(raw: string): number | null {
	if (raw.trim() === "") return null;
	const amount = Number(raw.replace(/^\$/, ""));
	return Number.isFinite(amount) && amount >= 0 ? Math.round(amount * 100) / 100 : null;
}

/**
 * The voice fronts a desk can run, in the order Live calls lists them, each
 * under the provider it comes from and once for every login or key that can
 * run it, told apart by who pays (the per-minute prices are the ones the
 * Voice budget counts).
 */
export const FRONTS = [
	{
		id: "grok",
		name: "Grok Voice",
		group: "xAI",
		connect: "Connect xAI or sign in to Grok above",
		voices: ["eve", "ara", "rex", "sal", "leo"],
		logins: [
			{ id: "xai-subscription", detail: "Included with your Grok subscription" },
			{ id: "xai", detail: "$0.08 a minute on your xAI key" },
		],
	},
	{
		id: "gpt-live",
		name: "GPT-Live",
		group: "OpenAI",
		connect: "Sign in with ChatGPT above",
		voices: ["cove", "juniper", "maple", "spruce", "ember", "vale", "breeze", "sol"],
		logins: [{ id: "openai-codex", detail: "Included with your ChatGPT subscription" }],
	},
	{
		id: "openai",
		name: "GPT Realtime",
		group: "OpenAI",
		// A ChatGPT sign-in does not run it.
		connect: "Add an OpenAI API key above",
		voices: ["marin", "cedar", "alloy", "ash", "ballad", "coral", "echo", "sage", "shimmer", "verse"],
		logins: [{ id: "openai", detail: "About $0.10 a minute on your OpenAI key" }],
	},
] as const;

/** Every login or key some front can run on. */
export const FRONT_LOGINS: readonly string[] = FRONTS.flatMap((one) => one.logins.map((login) => login.id));

export type FrontSetting = { provider: string; credential?: string; voices: Record<string, string> };

/** Off in Settings: the picker's last choice for Live calls and for Images, never the default. */
export const OFF = "off";

/** The Images picker's Off, in the picker's own `provider|model` ids. */
export const IMAGES_OFF_CHOICE: PickerChoice = { id: pickId(OFF, undefined), name: "Off" };

/** The pipeline: calls are heard, answered and spoken one step at a time, as before the front. */
export const LEGACY = "legacy";

/**
 * The voice front `settings.voice` holds. Without one, Grok Voice fronts
 * calls (the desk falls back to the pipeline when no xAI login or key is
 * connected); `off` offers no calls; `legacy`, or a provider this window
 * does not know, is the pipeline.
 */
export function frontOf(voice: unknown): FrontSetting {
	const fallback: FrontSetting = { provider: "grok", voices: {} };
	if (voice === null || typeof voice !== "object" || Array.isArray(voice)) return fallback;
	const front = (voice as Record<string, unknown>)["front"];
	if (front === null || typeof front !== "object" || Array.isArray(front)) return fallback;
	const { provider, credential, voices } = front as Record<string, unknown>;
	const named: Record<string, string> = {};
	if (voices !== null && typeof voices === "object" && !Array.isArray(voices)) {
		for (const [key, value] of Object.entries(voices)) if (typeof value === "string") named[key] = value;
	}
	const known =
		typeof provider !== "string" || provider === "grok"
			? "grok"
			: provider === OFF || FRONTS.some((one) => one.id === provider)
				? provider
				: LEGACY;
	return { provider: known, ...(typeof credential === "string" ? { credential } : {}), voices: named };
}

/**
 * `settings.voice` with the front changed, the rest kept. Legacy voice keeps
 * the voices picked, so going back to the front finds them; the default, Grok
 * on whatever xAI login is connected with nothing picked, removes the key.
 */
export function frontPatch(current: unknown, next: { provider: string; credential?: string; voice?: { provider: string; voice: string } }): Record<string, unknown> | null {
	const voice: Record<string, unknown> =
		current !== null && typeof current === "object" && !Array.isArray(current) ? { ...(current as Record<string, unknown>) } : {};
	const before = frontOf(current);
	const voices = { ...before.voices, ...(next.voice ? { [next.voice.provider]: next.voice.voice } : {}) };
	if (next.provider === "grok" && next.credential === undefined && Object.keys(voices).length === 0) delete voice["front"];
	else voice["front"] = { provider: next.provider, ...(next.credential !== undefined ? { credential: next.credential } : {}), voices };
	return Object.keys(voice).length === 0 ? null : voice;
}
