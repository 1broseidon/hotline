import { useEffect, useState } from "react";
import type { CapabilityJob, CapabilityOptions } from "../generated/contract";
import { Refusal } from "../ui/Refusal";
import { Picker } from "../ui/Menu";
import { wire } from "../wire";
import {
	AUTOMATIC,
	choicesFor,
	currentId,
	parseCap,
	spentText,
	splitPickId,
	stored,
	voiceChoices,
	voiceOf,
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

	const speaking = options.tts;
	const voices = voiceChoices(speaking);

	return (
		<section>
			<h3 className="group-title">Use for</h3>
			<div className="grouped use-for">
				<JobRow title="Images" detail="Pictures teammates draw" job={options.images}>
					<Picker
						value={currentId(options.images)}
						choices={choicesFor(options.images)}
						placeholder="Automatic"
						label="Model for images"
						onChange={(id) => {
							const next = pickModel(id);
							write({ images: next === null ? null : stored(next.provider, next.model) });
						}}
					/>
				</JobRow>
				<JobRow title="Voice · hearing" detail="Turns what you say into text" job={options.stt}>
					<Picker
						value={currentId(options.stt)}
						choices={choicesFor(options.stt)}
						placeholder="Automatic"
						label="Model for hearing you"
						onChange={(id) => setVoice("stt", pickModel(id))}
					/>
				</JobRow>
				<JobRow title="Voice · speaking" detail="Speaks the answer in a call" job={speaking}>
					<Picker
						value={currentId(speaking)}
						choices={choicesFor(speaking)}
						placeholder="Automatic"
						label="Model for speaking"
						onChange={(id) => setVoice("tts", pickModel(id))}
					/>
					{voices.length > 1 && (
						<Picker
							value={speaking.selected?.voice ?? AUTOMATIC}
							choices={voices}
							placeholder={voiceOf(speaking) ?? "Voice"}
							label="Voice"
							onChange={(voiceId) => {
								// A voice belongs to a provider, so naming one while the
								// provider is Automatic pins the one Automatic chose.
								const pick = speaking.selected ?? speaking.automatic;
								if (pick === undefined) return;
								setVoice("tts", stored(pick.providerId, pick.modelId, voiceId === AUTOMATIC ? undefined : voiceId));
							}}
						/>
					)}
				</JobRow>
				<JobRow title="Voice · call assistant" detail="Answers while you talk, and hands work to teammates" job={options.dispatcher}>
					<Picker
						value={currentId(options.dispatcher)}
						choices={choicesFor(options.dispatcher)}
						placeholder="Automatic"
						label="Model for the call assistant"
						onChange={(id) => setVoice("dispatcher", pickModel(id))}
					/>
				</JobRow>
				<SpendingRow spending={options.spending} onWrite={write} />
			</div>
			<p className="group-hint">Automatic uses the first connected provider that can do the job. One cap covers images and voice; zero turns them off.</p>
			{refusal !== null && <Refusal message={refusal} />}
		</section>
	);
}

/** A job's row: its name and what it is for, then its pickers, or the sentence that says what to connect. */
function JobRow({ title, detail, job, children }: { title: string; detail: string; job: CapabilityJob; children: React.ReactNode }) {
	const nothing = job.options.length === 0;
	return (
		<div className="group-row">
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
		<div className="group-row flex-wrap">
			<span className="group-row-text" style={{ minWidth: "14rem" }}>
				<span className="group-row-title">Spending</span>
				<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
					{spentText(spending)}
				</span>
			</span>
			<span className="flex shrink-0 items-center gap-2 text-sm text-ink-2">
				<Cap label="Daily cap" value={spending.dayUsd} unit="a day" onCommit={(dayUsd) => onWrite({ spending: { dayUsd, monthUsd: spending.monthUsd } })} />
				<Cap label="Monthly cap" value={spending.monthUsd} unit="a month" onCommit={(monthUsd) => onWrite({ spending: { dayUsd: spending.dayUsd, monthUsd } })} />
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
