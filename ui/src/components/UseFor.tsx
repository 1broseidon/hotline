import { useEffect, useState } from "react";
import type { CapabilityJob, CapabilityOptions } from "../generated/contract";
import { Refusal } from "../ui/Refusal";
import { Picker } from "../ui/Menu";
import { wire } from "../wire";
import { ChevronDownIcon } from "../icons";
import { deviceTranscription } from "../voice/transcription";
import { setHearOnThisMac, useHearOnThisMac } from "../voice/hearing";

/** The Hearing picker's own choice: this Mac, not a provider. Provider ids never start with a bar. */
const ON_THIS_MAC = "|this-mac";
import {
	AUTOMATIC,
	carriedEffort,
	choicesFor,
	currentId,
	effortChoices,
	effortsOf,
	parseCap,
	type PickerChoice,
	shortModel,
	spentText,
	splitPickId,
	stored,
	voicePatch,
	type Stored,
} from "../useFor";

/**
 * Which connected provider and model does each job, and the one cap on what
 * they may spend. It sits under Providers' connections because it is about
 * them: a picker lists only what is already connected, grouped by provider,
 * and Automatic, the first choice, is the key being absent — the desk picks
 * the first provider that can. A job nothing connected can do says what to
 * connect instead of offering a picker.
 *
 * Every write is one `settings.update` of the whole setting, so the voice
 * object is merged from what the room holds and not rebuilt from the
 * options, which would drop the caps and the fallback.
 */
export function UseFor({
	options,
	voice,
	onChanged,
}: {
	options: CapabilityOptions;
	/** `settings.voice` as written, so a write to one job keeps the rest. */
	voice: unknown;
	/** The options are stale: ask for them again. */
	onChanged(): void;
}) {
	const [refusal, setRefusal] = useState<string | null>(null);

	const write = (patch: Record<string, unknown>) => {
		setRefusal(null);
		wire
			.command("settings.update", { patch })
			.then(onChanged)
			.catch((error: Error) => setRefusal(error.message));
	};

	const setVoice = (job: "stt" | "tts" | "dispatcher", choice: Stored | null) =>
		write({ voice: voicePatch(voice, job, choice) });

	const pickModel = (id: string): Stored | null => {
		if (id === AUTOMATIC) return null;
		const { providerId, modelId } = splitPickId(id);
		return stored(providerId, modelId);
	};

	const [more, setMore] = useState(false);
	const hearsHere = useSpeechOnThisMac();
	const hearHere = useHearOnThisMac();
	const speaking = options.tts;
	const speakingNow = speaking.selected ?? speaking.automatic;
	const hearingNow = options.stt.selected ?? options.stt.automatic;

	// One voice pick sets who speaks and, when that provider can also hear,
	// who listens: a call is one provider unless the owner splits it below.
	const pickVoice = (id: string) => {
		if (id === AUTOMATIC) return write({ voice: voicePatch(voicePatch(voice, "tts", null), "stt", null) });
		const { providerId, modelId, voiceId } = splitVoiceId(id);
		let next = voicePatch(voice, "tts", stored(providerId, modelId, voiceId));
		if (options.stt.options.some((one) => one.providerId === providerId)) next = voicePatch(next, "stt", stored(providerId, undefined));
		write({ voice: next });
	};

	return (
		<section>
			<h3 className="group-title">Use for</h3>
			<div className="grouped use-for">
				<JobRow title="Images" detail={inUse(options.images)} job={options.images}>
					<Picker
						value={currentId(options.images)}
						choices={shortChoices(options.images)}
						placeholder="Automatic"
						label="Model for images"
						onChange={(id) => {
							const next = pickModel(id);
							write({ images: next === null ? null : stored(next.provider, next.model) });
						}}
					/>
				</JobRow>
				<JobRow
					title="Voice"
					detail={
						speakingNow === undefined
							? "Talk to the desk"
							: `${voiceName(speakingNow.voice) ?? "Default voice"} on ${speakingNow.providerName}${
									hearingNow !== undefined && hearingNow.providerId !== speakingNow.providerId ? `, hearing through ${hearingNow.providerName}` : ""
								}`
					}
					job={speaking}
				>
					<Picker
						value={currentVoiceId(speaking)}
						choices={allVoices(speaking)}
						placeholder="Automatic"
						label="Voice"
						onChange={pickVoice}
					/>
					<button
						type="button"
						className="control btn-icon"
						aria-expanded={more}
						aria-label="More voice settings"
						title="Hearing and call assistant"
						onClick={() => setMore((was) => !was)}
					>
						<ChevronDownIcon className={more ? "rotate-180" : ""} />
					</button>
				</JobRow>
				{more && (
					<>
						{hearsHere ? (
							<div className="group-row use-for-nested">
								<span className="group-row-text">
									<span className="group-row-title">Hearing</span>
									<span className="group-row-detail">
										{hearHere
											? `Free and private on calls from this Mac${hearingNow !== undefined ? `; other devices use ${hearingNow.providerName}` : ""}`
											: "Turns what you say into text"}
									</span>
								</span>
								<span className="flex shrink-0 items-center gap-1">
									<Picker
										value={hearHere ? ON_THIS_MAC : currentId(options.stt)}
										choices={[{ id: ON_THIS_MAC, name: "On this Mac", group: "This Mac" }, ...shortChoices(options.stt)]}
										placeholder="Automatic"
										label="How calls from this Mac hear you"
										onChange={(id) => {
											if (id === ON_THIS_MAC) return setHearOnThisMac(true);
											setHearOnThisMac(false);
											setVoice("stt", pickModel(id));
										}}
									/>
								</span>
							</div>
						) : (
							<JobRow title="Hearing" detail="Turns what you say into text" job={options.stt} nested>
								<Picker
									value={currentId(options.stt)}
									choices={shortChoices(options.stt)}
									placeholder="Automatic"
									label="Model for hearing you"
									onChange={(id) => setVoice("stt", pickModel(id))}
								/>
							</JobRow>
						)}
						<JobRow title="Call assistant" detail="Answers while you talk and hands work to teammates" job={options.dispatcher} nested>
							<Picker
								value={currentId(options.dispatcher)}
								choices={shortChoices(options.dispatcher)}
								placeholder="Automatic"
								label="Model for the call assistant"
								onChange={(id) => {
									const next = pickModel(id);
									setVoice("dispatcher", next === null ? null : stored(next.provider, next.model, undefined, carriedEffort(options.dispatcher, next.provider, next.model)));
								}}
							/>
							{effortsOf(options.dispatcher).length > 0 && (
								<Picker
									value={options.dispatcher.selected?.effort ?? AUTOMATIC}
									choices={effortChoices(options.dispatcher)}
									placeholder="Default thinking"
									label="Call assistant thinking"
									onChange={(id) => {
										const selected = options.dispatcher.selected;
										if (selected === undefined) return;
										setVoice("dispatcher", stored(selected.providerId, selected.modelId, undefined, id === AUTOMATIC ? undefined : id));
									}}
								/>
							)}
						</JobRow>
					</>
				)}
				<SpendingRow spending={options.spending} onWrite={write} />
			</div>
			<p className="group-hint">Automatic picks the first eligible connected provider, subscriptions before paid keys. Dollar limits cover paid images and voice; zero disables paid usage. Subscription limits apply separately.</p>
			{refusal !== null && <Refusal message={refusal} />}
		</section>
	);
}

/** Whether a call placed from this window is heard by this Mac itself (see voice/call.ts), asked without a prompt. */
function useSpeechOnThisMac(): boolean {
	const [available, setAvailable] = useState(false);
	useEffect(() => {
		let current = true;
		void deviceTranscription()?.capability().then((capability) => {
			if (current) setAvailable(capability.available);
		});
		return () => {
			current = false;
		};
	}, []);
	return available;
}

/** What a job runs on now, as its row's second line. */
function inUse(job: CapabilityJob): string {
	const pick = job.selected ?? job.automatic;
	if (pick === undefined) return job.options.length > 0 ? "Choose a provider for images" : "Nothing connected can do this";
	return [pick.providerName, pick.modelId === undefined ? undefined : shortModel(pick.modelId)].filter(Boolean).join(" · ");
}

/** The picker's own words stay short: the row's second line says what Automatic comes to. */
function shortChoices(job: CapabilityJob): PickerChoice[] {
	return choicesFor(job).map((choice) => (choice.id === AUTOMATIC ? { ...choice, name: "Automatic" } : choice));
}

/** Voices read as names: "eve" is Eve. */
function voiceName(voice: string | undefined): string | undefined {
	return voice === undefined ? undefined : voice.charAt(0).toUpperCase() + voice.slice(1);
}

const VOICE_SEPARATOR = "\u001f";

function voiceId(providerId: string, modelId: string, voiceId: string): string {
	return [providerId, modelId, voiceId].join(VOICE_SEPARATOR);
}

function splitVoiceId(id: string): { providerId: string; modelId: string; voiceId: string } {
	const [providerId = "", modelId = "", voiceId = ""] = id.split(VOICE_SEPARATOR);
	return { providerId, modelId, voiceId };
}

/**
 * Every voice of every connected speaking model, under its provider, and
 * under the model too where a provider has several: "OpenRouter · Grok Voice TTS 1.0".
 */
function allVoices(job: CapabilityJob): PickerChoice[] {
	const choices: PickerChoice[] = [{ id: AUTOMATIC, name: "Automatic" }];
	for (const provider of job.options) {
		for (const model of provider.models) {
			const group = provider.models.length > 1 ? `${provider.providerName} · ${model.label ?? shortModel(model.id)}` : provider.providerName;
			for (const voice of model.voices ?? []) {
				choices.push({ id: voiceId(provider.providerId, model.id, voice), name: voiceName(voice) ?? voice, group });
			}
		}
	}
	return choices;
}

function currentVoiceId(job: CapabilityJob): string {
	const selected = job.selected;
	if (selected === undefined) return AUTOMATIC;
	const provider = job.options.find((one) => one.providerId === selected.providerId);
	const model = provider?.models.find((one) => one.id === selected.modelId) ?? provider?.models[0];
	const voice = selected.voice ?? model?.voices?.[0];
	return model === undefined || voice === undefined ? AUTOMATIC : voiceId(selected.providerId, model.id, voice);
}

/** A job's row: its name and what it is for, then its pickers, or the sentence that says what to connect. */
function JobRow({
	title,
	detail,
	job,
	nested = false,
	children,
}: {
	title: string;
	detail: string;
	job: CapabilityJob;
	nested?: boolean;
	children: React.ReactNode;
}) {
	const nothing = job.options.length === 0;
	return (
		<div className={nested ? "group-row use-for-nested" : "group-row"}>
			<span className="group-row-text">
				<span className="group-row-title">{title}</span>
				<span className="group-row-detail" style={nothing ? { whiteSpace: "normal" } : undefined}>
					{nothing ? (job.unavailable ?? detail) : detail}
				</span>
			</span>
			{!nothing && <span className="flex shrink-0 items-center gap-1">{children}</span>}
		</div>
	);
}

function SpendingRow({ spending, onWrite }: { spending: CapabilityOptions["spending"]; onWrite(patch: Record<string, unknown>): void }) {
	return (
		<div className="group-row">
			<span className="group-row-text">
				<span className="group-row-title">Spending limit</span>
				<span className="group-row-detail">{spentText(spending)}</span>
			</span>
			<span className="flex shrink-0 items-center gap-3 text-sm text-ink-3">
				<Cap label="Daily limit" value={spending.dayUsd} unit="/ day" onCommit={(dayUsd) => onWrite({ spending: { dayUsd, monthUsd: spending.monthUsd } })} />
				<Cap label="Monthly limit" value={spending.monthUsd} unit="/ month" onCommit={(monthUsd) => onWrite({ spending: { dayUsd: spending.dayUsd, monthUsd } })} />
			</span>
		</div>
	);
}

/** A dollar amount typed in: saved when you leave it or press Enter, put back if it is not a cap. */
function Cap({ label, value, unit, onCommit }: { label: string; value: number; unit: string; onCommit(usd: number): void }) {
	const [text, setText] = useState(value.toFixed(2));

	useEffect(() => {
		setText(value.toFixed(2));
	}, [value]);

	const commit = () => {
		const next = parseCap(text);
		if (next === null) {
			setText(value.toFixed(2));
			return;
		}
		setText(next.toFixed(2));
		if (next !== value) onCommit(next);
	};

	return (
		<label className="flex items-center gap-1">
			<span className="text-ink-3">$</span>
			<input
				type="text"
				inputMode="decimal"
				className="field w-16 text-right"
				aria-label={label}
				value={text}
				onChange={(event) => setText(event.target.value)}
				onBlur={commit}
				onKeyDown={(event) => {
					if (event.key === "Enter") event.currentTarget.blur();
				}}
			/>
			{unit}
		</label>
	);
}
