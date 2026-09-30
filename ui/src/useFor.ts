import type { CapabilityJob, CapabilityOptions, CapabilityPick, CapabilityProvider } from "./generated/contract";

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
export function automaticText(job: CapabilityJob): string {
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

export function selectedId(pick: CapabilityPick | undefined): string {
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

/** The voices the speaking row offers: those of the model in use, automatic's included. */
export function voicesOf(job: CapabilityJob): string[] {
	const pick = job.selected ?? job.automatic;
	if (pick === undefined) return [];
	const provider = job.options.find((one) => one.providerId === pick.providerId);
	const model = provider?.models.find((one) => one.id === pick.modelId) ?? provider?.models[0];
	return model?.voices ?? [];
}

/** The voice in use: the owner's, else the provider's own. */
export function voiceOf(job: CapabilityJob): string | undefined {
	return job.selected?.voice ?? (job.selected === undefined ? job.automatic?.voice : undefined);
}

export function voiceChoices(job: CapabilityJob): PickerChoice[] {
	const voices = voicesOf(job);
	const own = voices[0];
	const named = job.selected?.voice;
	const choices: PickerChoice[] = [{ id: AUTOMATIC, name: own === undefined ? "Default voice" : `Default · ${own}` }];
	for (const voice of voices) choices.push({ id: voice, name: voice });
	if (named !== undefined && !voices.includes(named)) choices.push({ id: named, name: named });
	return choices;
}

/** A choice the desk stores: `{provider, model?, voice?}`. */
export type Stored = { provider: string; model?: string; voice?: string };

export function stored(providerId: string, modelId: string | undefined, voice?: string): Stored {
	return {
		provider: providerId,
		...(modelId !== undefined ? { model: modelId } : {}),
		...(voice !== undefined ? { voice } : {}),
	};
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

/** Whether a provider offers anything in any job, for the tags on its connection. */
export function tagsFor(options: CapabilityOptions, providerId: string): string[] {
	const offers = (providers: CapabilityProvider[]) => providers.some((one) => one.providerId === providerId);
	const tags: string[] = [];
	if (offers(options.dispatcher.options)) tags.push("Chat");
	if (offers(options.images.options)) tags.push("Images");
	if (offers(options.stt.options) || offers(options.tts.options)) tags.push("Voice");
	return tags;
}

/** Dollars as a person says them; a spend too small to show is not zero. */
export function usd(amount: number): string {
	if (amount > 0 && amount < 0.005) return "<$0.01";
	return `$${amount.toFixed(2)}`;
}

export function spentText(spending: CapabilityOptions["spending"]): string {
	const text = `${usd(spending.spentDayUsd)} spent today · ${usd(spending.spentMonthUsd)} this month`;
	return spending.unavailable === undefined ? text : `${text} (some spending could not be read)`;
}

/** A cap typed into a field: a finite, non-negative dollar amount, else nothing. */
export function parseCap(raw: string): number | null {
	if (raw.trim() === "") return null;
	const amount = Number(raw.replace(/^\$/, ""));
	return Number.isFinite(amount) && amount >= 0 ? Math.round(amount * 100) / 100 : null;
}
