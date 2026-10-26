import type { CapabilityJob, CapabilityLiveModel, CapabilityModel, CapabilityOptions, CapabilityPick, CapabilityProvider } from "./generated/contract";

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
 * A speaking model as the Speech row names it: its label as a name
 * ("phonon" is Phonon), else its id as the provider spells it.
 */
function speechModelName(model: CapabilityModel): string {
	return model.label === undefined ? shortModel(model.id) : voiceLabel(model.label);
}

/**
 * The Speech picker: every voice of every connected speaking model, under
 * its provider, each named with its model ("Phonon · Zoey"), so the closed
 * picker says both. A model's own voice is named for the model alone.
 */
export function speechChoices(job: CapabilityJob): PickerChoice[] {
	const choices: PickerChoice[] = [{ id: AUTOMATIC, name: "Automatic" }];
	for (const provider of job.options) {
		for (const model of provider.models) {
			for (const voice of model.voices ?? []) {
				const name = voice === OWN_VOICE ? speechModelName(model) : `${speechModelName(model)} · ${voiceLabel(voice)}`;
				choices.push({ id: voiceId(provider.providerId, model.id, voice), name, group: provider.providerName });
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

/** What a Transcription or Speech row runs on now, and where: the pick, else what Automatic comes to. */
type RunsOn = { provider: CapabilityProvider | undefined; pick: CapabilityPick; model: CapabilityModel | undefined };

function runsOn(job: CapabilityJob): RunsOn | undefined {
	const pick = job.selected ?? job.automatic;
	if (pick === undefined) return undefined;
	const provider = job.options.find((one) => one.providerId === pick.providerId);
	return { provider, pick, model: pickedModel(job, pick) };
}

/** The desk's own models' provider, whose models are named family first ("Parakeet TDT 0.6B"). */
const ON_THE_DESK = "local";

/**
 * The Transcription row's second line: who hears, and what it costs
 * ("Parakeet · free", "Groq · $0.001 a minute"). A model on the desk is
 * named by its family, since the desk is not a provider anyone chose.
 */
export function transcriptionLine(job: CapabilityJob): string {
	const now = runsOn(job);
	if (now === undefined) return "";
	const who =
		now.pick.providerId === ON_THE_DESK && now.model !== undefined ? (modelName(now.model).split(" ")[0] ?? now.pick.providerName) : now.pick.providerName;
	return [who, now.provider?.cost].filter((part) => part !== undefined && part !== "").join(" · ");
}

/** The Speech row's second line: the model through its provider, and what it costs ("Phonon through Ollama · free"). */
export function speechLine(job: CapabilityJob): string {
	const now = runsOn(job);
	if (now === undefined) return "";
	const who = now.model === undefined ? now.pick.providerName : `${speechModelName(now.model)} through ${now.pick.providerName}`;
	return [who, now.provider?.cost].filter((part) => part !== undefined && part !== "").join(" · ");
}

/**
 * The small tag beside a Transcription or Speech row whose model runs on
 * the desk's own machine, named as the desk names it ("on this Mac", "on
 * this computer"); none for a model reached over the network.
 */
export function whereTag(job: CapabilityJob, deviceName: string | undefined): string | undefined {
	return runsOn(job)?.provider?.onDevice === true ? `on ${deviceName ?? "the desk"}` : undefined;
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
	// A login that runs a live model does voice even with no speech job (the ChatGPT sign-in).
	const live = (options.live ?? []).some((one) => (SPEECH_FROM[one.credential] ?? one.credential) === providerId);
	if (offers(options.stt.options) || offers(options.tts.options) || live) tags.push("Voice");
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

/** Off in Settings: the picker's last choice for Calls and for Images, never the default. */
export const OFF = "off";

/** The Images picker's Off, in the picker's own `provider|model` ids. */
export const IMAGES_OFF_CHOICE: PickerChoice = { id: pickId(OFF, undefined), name: "Off" };

/**
 * Turn by turn: the teammate answers on its own model, and the desk
 * transcribes what is said and speaks the reply. Desks up to 0.43 wrote
 * `legacy` for it, and phones of their time still do, so that reads as
 * this too.
 */
export const TURN_BY_TURN = "turn-by-turn";

/**
 * The live models the desk has adapters for, by the name `voice.front`
 * gives each, with the name it shows while its provider is not connected
 * (the desk's `capabilities.options.live` names the connected ones).
 */
const LIVE_NAMES: Record<string, string> = { grok: "Grok Voice", "gpt-live": "GPT-Live", openai: "GPT Realtime" };

export type FrontSetting = { provider: string; credential?: string; voices: Record<string, string> };

/**
 * The call model `settings.voice` holds. Without one, Grok Voice fronts
 * calls; `off` offers no calls; `turn-by-turn`, `legacy`, or a model this
 * window does not know, is Turn by turn, as the desk reads it.
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
			: provider === OFF || provider in LIVE_NAMES
				? provider
				: TURN_BY_TURN;
	return { provider: known, ...(typeof credential === "string" ? { credential } : {}), voices: named };
}

/** What a call model's choice shows beneath it. */
export type CallKind = "live" | "turn-by-turn" | "off";

export function callKind(front: FrontSetting): CallKind {
	return front.provider === OFF ? "off" : front.provider === TURN_BY_TURN ? "turn-by-turn" : "live";
}

/**
 * The rows of Settings › Calls for each choice: a live model hears and
 * speaks itself, so it has only its voice; Turn by turn has the desk's
 * Transcription and Speech; Off has nothing to set.
 */
export function callRows(kind: CallKind): string[] {
	switch (kind) {
		case "live":
			return ["Model", "Voice", "Call assistant"];
		case "turn-by-turn":
			return ["Model", "Transcription", "Speech", "Call assistant"];
		case "off":
			return ["Model"];
	}
}

/** What the Model row says of each choice, and the hint under the group. */
export const CALL_WORDS: Record<CallKind, { detail: string; hint?: string }> = {
	live: { detail: "Hears, talks and uses tools in one stream", hint: "Talk over it to interrupt." },
	"turn-by-turn": { detail: "The teammate answers; the desk transcribes and speaks", hint: "Tap to interrupt." },
	off: { detail: "No calls, to the desk or a teammate" },
};

/** A live model's picker id: the model and the login it runs on, `grok|xai`. */
function liveId(front: string, credential: string): string {
	return `${front}${SEPARATOR}${credential}`;
}

/** The connected live model the setting names: its login, else the first that runs it. */
export function chosenLive(live: CapabilityLiveModel[], front: FrontSetting): CapabilityLiveModel | undefined {
	const running = live.filter((one) => one.front === front.provider);
	return running.find((one) => one.credential === front.credential) ?? running[0];
}

/**
 * The Model picker: Live, the realtime models of connected providers, each
 * once per login or key that runs it; then Turn by turn; then Off. A live
 * model the owner chose whose provider is no longer connected stays listed
 * as such, so the picker never claims calls run on something they do not.
 */
export function callModelChoices(live: CapabilityLiveModel[], front: FrontSetting): PickerChoice[] {
	const choices: PickerChoice[] = live.map((one) => ({ id: liveId(one.front, one.credential), name: one.name, detail: one.providerName, group: "Live" }));
	if (callKind(front) === "live" && chosenLive(live, front) === undefined) {
		choices.push({ id: liveId(front.provider, front.credential ?? ""), name: LIVE_NAMES[front.provider] ?? front.provider, detail: "Not connected", group: "Live" });
	}
	choices.push({ id: TURN_BY_TURN, name: "Turn by turn", detail: "teammate's own model", group: "Turn by turn" });
	choices.push({ id: OFF, name: "Off", detail: "No calls", group: "No calls" });
	return choices;
}

/** The Model picker's current choice. */
export function callModelId(live: CapabilityLiveModel[], front: FrontSetting): string {
	const kind = callKind(front);
	if (kind !== "live") return kind === "off" ? OFF : TURN_BY_TURN;
	const chosen = chosenLive(live, front);
	return chosen === undefined ? liveId(front.provider, front.credential ?? "") : liveId(chosen.front, chosen.credential);
}

/** What picking a Model choice writes as the front: Turn by turn and Off by name, a live model with its login. */
export function callModelPick(id: string): { provider: string; credential?: string } {
	if (id === TURN_BY_TURN || id === OFF) return { provider: id };
	const at = id.indexOf(SEPARATOR);
	return { provider: id.slice(0, at), credential: id.slice(at + 1) };
}

/**
 * `settings.voice` with the front changed, the rest kept. Turn by turn keeps
 * the voices picked, so going back to a live model finds them; the default,
 * Grok on whatever xAI login is connected with nothing picked, removes the key.
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
