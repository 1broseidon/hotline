import { type ReactNode, useEffect, useState } from "react";
import type { CapabilityJob, CapabilityOptions } from "../generated/contract";
import { Refusal } from "../ui/Refusal";
import { Picker } from "../ui/Menu";
import { wire } from "../wire";
import { deviceTranscription } from "../voice/transcription";
import { setHearOnThisMac, useHearOnThisMac } from "../voice/hearing";
import { Fold } from "../ui/Fold";
import { DeskModels } from "./DeskModels";

/** The Transcription picker's own choice: this Mac, not a provider. Provider ids never start with a bar. */
const ON_THIS_MAC = "|this-mac";
/** The desk's own models, as the core names their provider. */
const ON_THE_DESK = "local";
import {
	AUTOMATIC,
	carriedEffort,
	choicesFor,
	currentId,
	effortChoices,
	effortsOf,
	FRONTS,
	FRONT_LOGINS,
	IMAGES_OFF_CHOICE,
	LEGACY,
	OFF,
	frontOf,
	frontPatch,
	listenForOf,
	listenForPatch,
	parseWords,
	type FrontSetting,
	type PickerChoice,
	shortModel,
	splitPickId,
	stored,
	voicePatch,
	type Stored,
} from "../useFor";

/**
 * Which connected provider and model does each job (what they may spend is
 * Budgets, below). It sits under Providers' connections because it is about
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
	connected = [],
	onChanged,
}: {
	options: CapabilityOptions;
	/** The providers with a live login or key, for the fronts no speech job lists (a ChatGPT sign-in). */
	connected?: string[];
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
	const hearsOnDesk = hearingNow?.providerId === ON_THE_DESK;

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
		<>
			<section>
				<h3 className="group-title">Use for</h3>
				<div className="grouped use-for">
					<JobRow title="Images" detail={options.images.selected?.providerId === OFF ? "Teammates cannot make images" : inUse(options.images)} job={options.images}>
						<Picker
							value={currentId(options.images)}
							choices={withOff(shortChoices(options.images))}
							placeholder="Automatic"
							label="Model for images"
							onChange={(id) => {
								const next = pickModel(id);
								write({ images: next === null ? null : stored(next.provider, next.model) });
							}}
						/>
					</JobRow>
					{/* Transcription and the free models, folded under one row that says what transcribes you now. */}
					<button
						type="button"
						className="nt-fold-row nt-fold-head"
						aria-expanded={more}
						aria-label="Transcription"
						onClick={() => setMore((was) => !was)}
					>
						<span className="nt-fold-title">Transcription</span>
						<span className="nt-fold-value">
							{hearsHere && hearHere
								? "On this Mac, free and private"
								: hearsOnDesk
									? "On the desk, free and private"
									: hearingNow !== undefined
										? hearingNow.providerName
										: "None yet. Download a free speech model."}
						</span>
						<span className="nt-fold-action">{more ? "Done" : hearingNow === undefined && !hearsHere ? "Set up" : "Change"}</span>
					</button>
					{more && (
						<>
							{hearsHere ? (
								<div className="group-row use-for-nested">
									<span className="group-row-text">
										<span className="group-row-title">Transcribes with</span>
									</span>
									<span className="flex shrink-0 items-center gap-1">
										<Picker
											value={hearHere ? ON_THIS_MAC : currentId(options.stt)}
											choices={[{ id: ON_THIS_MAC, name: "On this Mac", group: "This Mac" }, ...shortChoices(options.stt)]}
											placeholder="Automatic"
											label="Transcription on calls from this Mac"
											onChange={(id) => {
												if (id === ON_THIS_MAC) return setHearOnThisMac(true);
												setHearOnThisMac(false);
												setVoice("stt", pickModel(id));
											}}
										/>
									</span>
								</div>
							) : (
								<JobRow title="Transcribes with" detail="" job={options.stt} nested>
									<Picker
										value={currentId(options.stt)}
										choices={shortChoices(options.stt)}
										placeholder="Automatic"
										label="Transcription model"
										onChange={(id) => setVoice("stt", pickModel(id))}
									/>
								</JobRow>
							)}
							<DeskModels onInstalledChanged={onChanged} />
							{options.stt.options.some((one) => one.providerId === ON_THE_DESK) && (
								<ListenFor saved={listenForOf(voice)} onSave={(words) => write({ voice: listenForPatch(voice, words) })} />
							)}
						</>
					)}
				</div>
			</section>
			<section>
				<LiveCalls
					front={frontOf(voice)}
					logins={[...new Set([...options.tts.options.map((one) => one.providerId), ...connected])].filter((id) => FRONT_LOGINS.includes(id))}
					write={(next) => write({ voice: frontPatch(voice, next) })}
				>
							<JobRow
								title="Voice"
								detail={
									speakingNow === undefined
										? "Talk to the desk"
										: `${voiceName(speakingNow.voice) ?? "Default voice"} on ${speakingNow.providerName}`
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
							</JobRow>
							<JobRow title="Call assistant" detail="Answers calls to the desk, and rewrites a teammate's reply for speech when needed." job={options.dispatcher} wrap>
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
				</LiveCalls>
				{refusal !== null && <Refusal message={refusal} />}
			</section>
		</>
	);
}

/** A front's picker id: the provider and the login it runs on, `grok|xai`. */
function frontId(provider: string, login: string): string {
	return `${provider}|${login}`;
}

/**
 * Who answers a live call, to the desk or to a teammate, picked as one model
 * from a list grouped by the provider it comes from. Grok Voice, the default,
 * and GPT Realtime are realtime models that talk with the person and run the
 * teammate's lookups themselves, and GPT-Live is the same agent with its
 * reasoning delegated to a backend model, all handing heavy work to the teammate. Each is listed once for every connected
 * login or key that can run it, since that decides who pays, or once with nothing connected so it
 * can be found. Legacy voice is the transcribe, route
 * and speak pipeline, and its own rows show only under it; the desk also
 * falls back on it by itself when the chosen front has nothing connected.
 */
function LiveCalls({
	front,
	logins,
	write,
	children,
}: {
	front: FrontSetting;
	/** The connected logins and keys that can run a front. */
	logins: string[];
	write(next: Parameters<typeof frontPatch>[1]): void;
	/** Legacy voice's own rows: the voice it speaks in and the call assistant. */
	children: ReactNode;
}) {
	const chosen = FRONTS.find((one) => one.id === front.provider);
	const usable = chosen?.logins.filter((one) => logins.includes(one.id)) ?? [];
	const login = usable.find((one) => one.id === front.credential) ?? usable[0];
	const voice = chosen === undefined ? undefined : (front.voices[chosen.id] ?? chosen.voices[0]);
	const choices = [
		...FRONTS.flatMap((one): PickerChoice[] => {
			const connected = one.logins.filter((via) => logins.includes(via.id));
			// Listed with nothing connected too, so it can be found; picking it does nothing until it is.
			if (connected.length === 0) return [{ id: frontId(one.id, ""), name: one.name, detail: one.connect, group: one.group }];
			return connected.map((via) => ({ id: frontId(one.id, via.id), name: one.name, detail: via.detail, group: one.group }));
		}),
		{ id: LEGACY, name: "Legacy voice", detail: "Hears you, thinks, then speaks, one step at a time", group: "Hotline" },
		{ id: OFF, name: "Off", detail: "No calls, to the desk or a teammate", group: "Hotline" },
	];
	const off = front.provider === OFF;
	const value = off ? OFF : chosen === undefined ? LEGACY : login === undefined ? "" : frontId(chosen.id, login.id);
	return (
		<>
			<h3 className="group-title">Live calls</h3>
			<div className="grouped use-for">
				<div className="group-row">
					<span className="group-row-text">
						<span className="group-row-title">Model</span>
						<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
							{off
								? "Calls are off."
								: chosen === undefined
								? "Calls hear you, think, then speak, one step at a time."
								: login === undefined
									? `${chosen.connect}. Until then, calls use Legacy voice.`
									: `A full agent: talks with you, looks things up and hands heavy work to your teammate. ${login.detail}.`}
						</span>
					</span>
					<Picker
						value={value}
						choices={choices}
						placeholder={off ? "Off" : chosen === undefined ? "Legacy voice" : chosen.name}
						label="Model for live calls"
						onChange={(id) => {
							if (id === LEGACY || id === OFF) return write({ provider: id });
							const at = id.indexOf("|");
							if (at === id.length - 1) return;
							write({ provider: id.slice(0, at), credential: id.slice(at + 1) });
						}}
					/>
				</div>
				{chosen === undefined && !off && children}
				{chosen !== undefined && (
					<div className="group-row use-for-nested">
						<span className="group-row-text">
							<span className="group-row-title">Voice</span>
						</span>
						<Picker
							value={voice ?? ""}
							choices={chosen.voices.map((id) => ({ id, name: voiceName(id) ?? id }))}
							placeholder="Voice"
							label={`Voice for ${chosen.name}`}
							onChange={(id) => write({ provider: chosen.id, ...(login !== undefined ? { credential: login.id } : {}), voice: { provider: chosen.id, voice: id } })}
						/>
					</div>
				)}
			</div>
		</>
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

/** Off, last: choicesFor lists a pick no provider offers as "Not connected", which Off is not. */
function withOff(choices: PickerChoice[]): PickerChoice[] {
	return [...choices.filter((choice) => choice.id !== IMAGES_OFF_CHOICE.id), IMAGES_OFF_CHOICE];
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

/**
 * A job's row: its name and what it is for, then its pickers, or the
 * sentence that says what to connect. `always` stays when there is nothing
 * to pick, so what is under the row can still be opened.
 */
/**
 * The words the desk's own models listen for besides teammates' names:
 * names and product words a small model would otherwise spell as the
 * nearest common word (voice.md, Hearing on the desk).
 */
function ListenFor({ saved, onSave }: { saved: string[]; onSave(words: string[]): void }) {
	const [open, setOpen] = useState(false);
	const [text, setText] = useState(saved.join(", "));
	const changed = parseWords(text).join("\n") !== saved.join("\n");
	return (
		<Fold
			title="Words to listen for"
			value={saved.length === 0 ? "None" : saved.join(", ")}
			action={saved.length === 0 ? "Add" : "Change"}
			open={open}
			onToggle={() => {
				setText(saved.join(", "));
				setOpen((was) => !was);
			}}
		>
			<form
				className="flex items-center gap-2"
				onSubmit={(event) => {
					event.preventDefault();
					onSave(parseWords(text));
					setOpen(false);
				}}
			>
				<input className="field min-w-0 flex-1" spellCheck={false} placeholder="Ophelia, Groq, Kubernetes" aria-label="Words to listen for" value={text} onChange={(event) => setText(event.target.value)} />
				<button type="submit" className="control btn btn-primary shrink-0" disabled={!changed}>
					Save
				</button>
			</form>
			<p className="hint">Parakeet listens for these, separated by commas, and for your teammates' names.</p>
		</Fold>
	);
}

function JobRow({
	title,
	detail,
	job,
	nested = false,
	wrap = false,
	always,
	children,
}: {
	title: string;
	detail: string;
	job: CapabilityJob;
	nested?: boolean;
	/** The detail is a sentence to read whole, not a value to clip. */
	wrap?: boolean;
	always?: React.ReactNode;
	children: React.ReactNode;
}) {
	const nothing = job.options.length === 0;
	return (
		<div className={nested ? "group-row use-for-nested" : "group-row"}>
			<span className="group-row-text">
				<span className="group-row-title">{title}</span>
				{(nothing ? (job.unavailable ?? detail) : detail) !== "" && (
					<span className="group-row-detail" style={nothing || wrap ? { whiteSpace: "normal" } : undefined}>
						{nothing ? (job.unavailable ?? detail) : detail}
					</span>
				)}
			</span>
			{(!nothing || always !== undefined) && (
				<span className="flex shrink-0 items-center gap-1">
					{!nothing && children}
					{always}
				</span>
			)}
		</div>
	);
}
