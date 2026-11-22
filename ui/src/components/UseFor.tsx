import { useEffect, useState } from "react";
import type { CapabilityJob, CapabilityOptions } from "../generated/contract";
import { Refusal } from "../ui/Refusal";
import { Picker } from "../ui/Menu";
import { wire } from "../wire";
import { deviceTranscription } from "../voice/transcription";
import { setHearOnThisMac, setInterruptBySpeaking, useHearOnThisMac, useInterruptBySpeaking } from "../voice/hearing";
import { Fold } from "../ui/Fold";
import {
	acknowledgeOf,
	acknowledgePatch,
	AUTOMATIC,
	CALL_WORDS,
	callFront,
	callKind,
	callModelChoices,
	callModelId,
	callModelPick,
	callRows,
	carriedEffort,
	chosenLive,
	choicesFor,
	currentId,
	currentVoiceId,
	effortChoices,
	effortsOf,
	frontPatch,
	IMAGES_OFF_CHOICE,
	listenForOf,
	listenForPatch,
	OFF,
	parseWords,
	type PickerChoice,
	shortModel,
	speechChoices,
	speechLine,
	splitPickId,
	splitVoiceId,
	stored,
	transcriptionLine,
	voiceLabel,
	voicePatch,
	whereTag,
	type Stored,
} from "../useFor";

/** The Transcription picker's own choice: this Mac's recognizer, not a provider. Provider ids never start with a bar. */
const ON_THIS_MAC = "|this-mac";
/** The desk's own models, as the core names their provider. */
const ON_THE_DESK = "local";

/**
 * Which connected provider and model makes images (what it may spend is
 * Budgets, below). It sits under Providers' connections because it is about
 * them: the picker lists only what is already connected, grouped by
 * provider, and Automatic, the first choice, is the key being absent — the
 * desk picks the first provider that can. How calls run is Settings › Voice
 * (`CallSettings`).
 */
export function UseFor({
	options,
	onChanged,
}: {
	options: CapabilityOptions;
	/** The options are stale: ask for them again. */
	onChanged(): void;
}) {
	const { write, refusal } = useWrite(onChanged);
	return (
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
			</div>
			{refusal !== null && <Refusal message={refusal} />}
		</section>
	);
}

/**
 * Every write is one `settings.update` of the whole setting, so the voice
 * object is merged from what the room holds and not rebuilt from the
 * options, which would drop the caps and the fallback.
 */
function useWrite(onChanged: () => void): { write(patch: Record<string, unknown>): void; refusal: string | null } {
	const [refusal, setRefusal] = useState<string | null>(null);
	const write = (patch: Record<string, unknown>) => {
		setRefusal(null);
		wire
			.command("settings.update", { patch })
			.then(onChanged)
			.catch((error: Error) => setRefusal(error.message));
	};
	return { write, refusal };
}

function pickModel(id: string): Stored | null {
	if (id === AUTOMATIC) return null;
	const { providerId, modelId } = splitPickId(id);
	return stored(providerId, modelId);
}

/**
 * How calls run, to the desk or to a teammate, and the rows that choice
 * has. A live model (Grok Voice, GPT-Live, GPT Realtime) hears, talks and
 * runs the teammate's lookups itself in one stream, so it has only its
 * voice; it is listed once for every connected login or key that runs it,
 * since that decides who pays, and never for a provider that is not
 * connected. Turn by turn is the teammate answering on its own model while
 * the desk transcribes and speaks, so it has Transcription and Speech, and
 * the words the desk's own models listen for once one is installed. The
 * call assistant answers calls to the desk either way. Settings › Voice
 * holds it, over the page of the desk's own models.
 */
export function CallSettings({
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
	const { write, refusal } = useWrite(onChanged);
	const live = options.live ?? [];
	const front = callFront(voice, live);
	const kind = callKind(front);
	const rows = callRows(kind);
	const chosen = chosenLive(live, front);
	const setVoice = (job: "stt" | "tts" | "dispatcher", choice: Stored | null) => write({ voice: voicePatch(voice, job, choice) });
	const words = CALL_WORDS[kind];
	const detail = kind === "live" && chosen === undefined ? "Not connected. Connect its provider above, or choose another." : words.detail;
	const onDesk = options.stt.options.some((one) => one.providerId === ON_THE_DESK);
	return (
		<section>
			<h3 className="group-title">Calls</h3>
			<div className="grouped use-for">
				<div className="group-row">
					<span className="group-row-text">
						<span className="group-row-title">Model</span>
						<span className="group-row-detail" style={{ whiteSpace: "normal" }}>{detail}</span>
					</span>
					<Picker
						value={callModelId(live, front)}
						choices={callModelChoices(live, front)}
						placeholder="Turn by turn"
						label="Model for calls"
						onChange={(id) => write({ voice: frontPatch(voice, callModelPick(id)) })}
					/>
				</div>
				{rows.includes("Voice") && chosen !== undefined && (
					<div className="group-row">
						<span className="group-row-text">
							<span className="group-row-title">Voice</span>
							<span className="group-row-detail">{`${voiceLabel(front.voices[chosen.front] ?? chosen.voices[0] ?? "")} on ${chosen.providerName}`}</span>
						</span>
						<Picker
							value={front.voices[chosen.front] ?? chosen.voices[0] ?? ""}
							choices={chosen.voices.map((id) => ({ id, name: voiceLabel(id) }))}
							placeholder="Voice"
							label={`Voice for ${chosen.name}`}
							onChange={(id) => write({ voice: frontPatch(voice, { provider: chosen.front, credential: chosen.credential, voice: { provider: chosen.front, voice: id } }) })}
						/>
					</div>
				)}
				{rows.includes("Transcription") && <Transcription options={options} voice={voice} write={write} />}
				{rows.includes("Speech") && (
					<JobRow title="Speech" detail={speechLine(options.tts)} tag={whereTag(options.tts, options.deviceName)} job={options.tts}>
						<Picker
							value={currentVoiceId(options.tts)}
							choices={speechChoices(options.tts)}
							placeholder="Automatic"
							label="Speech"
							onChange={(id) => {
								if (id === AUTOMATIC) return setVoice("tts", null);
								const { providerId, modelId, voiceId } = splitVoiceId(id);
								setVoice("tts", stored(providerId, modelId, voiceId));
							}}
						/>
					</JobRow>
				)}
				{rows.includes("Call assistant") && (
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
				)}
				{rows.includes("Acknowledge") && (
					// The call assistant says a short line in the teammate's voice while its model starts (voice.md).
					<label className="group-row group-row-choice" title="Covers the wait">
						<span className="group-row-text">
							<span className="group-row-title">Acknowledge while the teammate thinks</span>
						</span>
						<input
							type="checkbox"
							role="switch"
							className="switch"
							aria-label="Acknowledge while the teammate thinks"
							aria-checked={acknowledgeOf(voice)}
							checked={acknowledgeOf(voice)}
							onChange={(event) => write({ voice: acknowledgePatch(voice, event.target.checked) })}
						/>
					</label>
				)}
				{rows.includes("Interrupt") && <InterruptBySpeaking />}
				{rows.includes("Transcription") && onDesk && <ListenFor saved={listenForOf(voice)} onSave={(words) => write({ voice: listenForPatch(voice, words) })} />}
			</div>
			{words.hint !== undefined && <p className="group-hint">{words.hint}</p>}
			{refusal !== null && <Refusal message={refusal} />}
		</section>
	);
}

/**
 * Turn by turn's Transcription: the model that turns what is said into
 * words, which only Turn by turn uses; the desk's own free models are
 * downloaded on the Voice models page. This Mac's own recognizer is offered
 * too where the window has one (macOS's speech recognition, so only ever
 * on a Mac), for calls placed from this window.
 */
function Transcription({
	options,
	voice,
	write,
}: {
	options: CapabilityOptions;
	voice: unknown;
	write(patch: Record<string, unknown>): void;
}) {
	const hearsHere = useSpeechOnThisMac() !== null;
	const hearHere = useHearOnThisMac() && hearsHere;
	const job = options.stt;
	const choices: PickerChoice[] = [...(hearsHere ? [{ id: ON_THIS_MAC, name: "On this Mac", group: "This Mac" }] : []), ...shortChoices(job)];
	return (
		<JobRow
			title="Transcription"
			detail={hearHere ? "Apple speech recognition · free" : transcriptionLine(job)}
			tag={hearHere ? "on this Mac" : whereTag(job, options.deviceName)}
			job={job}
			ownChoice={hearsHere}
		>
			<Picker
				value={hearHere ? ON_THIS_MAC : currentId(job)}
				choices={choices}
				placeholder="Automatic"
				label="Transcription"
				onChange={(id) => {
					if (id === ON_THIS_MAC) return setHearOnThisMac(true);
					setHearOnThisMac(false);
					write({ voice: voicePatch(voice, "stt", pickModel(id)) });
				}}
			/>
		</JobRow>
	);
}

/**
 * Which of this Mac's engines would hear a call placed from this window
 * (see voice/call.ts), or null where none can; asked without a prompt.
 */
function useSpeechOnThisMac(): string | null {
	const [engine, setEngine] = useState<string | null>(null);
	useEffect(() => {
		let current = true;
		void deviceTranscription()?.capability().then((capability) => {
			if (current) setEngine(capability.available ? (capability.engine ?? "") : null);
		});
		return () => {
			current = false;
		};
	}, []);
	return engine;
}

/**
 * Talking over the teammate interrupts a Turn by turn call (voice/bargeIn.ts).
 * Offered only where it works: this Mac hears the call with its analyzer,
 * whose echo-cancelled channel it listens to. This computer's choice, on
 * until the person switches it off.
 */
function InterruptBySpeaking() {
	const engine = useSpeechOnThisMac();
	const on = useInterruptBySpeaking();
	if (!useHearOnThisMac() || engine !== "apple-analyzer") return null;
	return (
		<label className="group-row group-row-choice" title="Best with headphones">
			<span className="group-row-text">
				<span className="group-row-title">Interrupt by speaking</span>
			</span>
			<input
				type="checkbox"
				role="switch"
				className="switch"
				aria-label="Interrupt by speaking"
				aria-checked={on}
				checked={on}
				onChange={(event) => setInterruptBySpeaking(event.target.checked)}
			/>
		</label>
	);
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

/**
 * A job's row: its name and what it is for, then its pickers, or the
 * sentence that says what to connect.
 */
function JobRow({
	title,
	detail,
	tag,
	job,
	ownChoice = false,
	wrap = false,
	children,
}: {
	title: string;
	detail: string;
	/** A word or two beside the title: where the model runs ("on this Mac"). */
	tag?: string | undefined;
	job: CapabilityJob;
	/** The picker offers a choice of its own besides the job's providers (this Mac's recognizer). */
	ownChoice?: boolean;
	/** The detail is a sentence to read whole, not a value to clip. */
	wrap?: boolean;
	children: React.ReactNode;
}) {
	const nothing = job.options.length === 0 && !ownChoice;
	return (
		<div className="group-row">
			<span className="group-row-text">
				<span className="group-row-title">
					{title}
					{tag !== undefined && !nothing && <span className="use-for-tag">{tag}</span>}
				</span>
				{(nothing ? (job.unavailable ?? detail) : detail) !== "" && (
					<span className="group-row-detail" style={nothing || wrap ? { whiteSpace: "normal" } : undefined}>
						{nothing ? (job.unavailable ?? detail) : detail}
					</span>
				)}
			</span>
			{!nothing && <span className="flex shrink-0 items-center gap-1">{children}</span>}
		</div>
	);
}
