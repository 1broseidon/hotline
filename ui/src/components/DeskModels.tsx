import { Fragment, useEffect, useRef, useState } from "react";
import type { CapabilityOptions, SpeechModel } from "../generated/contract";
import { openLink } from "../native";
import { Refusal } from "../ui/Refusal";
import { noteDeskModels } from "../voice/desk";
import { busy, installedChanged, languageHint, megabytes, modelKind, modelLine, progress, usedBytes, usedForCalls } from "../voice/deskModels";
import { wire } from "../wire";

/** How often a download under way is asked after. */
const POLL_MS = 500;

type ModelCommand = "voice.model_install" | "voice.model_cancel" | "voice.model_remove";

/** The desk's own speech models as the desk last said, and the way to act on one. */
export type DeskModelsState = {
	/** Null until the desk answers. */
	models: SpeechModel[] | null;
	refusal: string | null;
	act(cmd: ModelCommand, modelId: string): void;
};

/**
 * The desk's own speech models, asked once and then again while a download
 * is under way, until it settles. Voice holds this for both its row and the
 * page under it, so a download keeps being followed whichever is open.
 */
export function useDeskModels(onInstalledChanged: () => void): DeskModelsState {
	const [models, setModels] = useState<SpeechModel[] | null>(null);
	const [refusal, setRefusal] = useState<string | null>(null);
	const shown = useRef<SpeechModel[] | null>(null);
	const changed = useRef(onInstalledChanged);
	changed.current = onInstalledChanged;

	const take = (next: SpeechModel[]) => {
		noteDeskModels(next);
		if (shown.current !== null && installedChanged(shown.current, next)) changed.current();
		shown.current = next;
		setModels(next);
	};

	// biome-ignore lint/correctness/useExhaustiveDependencies: take reads only refs and setters.
	useEffect(() => {
		void wire
			.command("voice.models", {})
			.then(take)
			.catch((error: Error) => setRefusal(error.message));
	}, []);

	// biome-ignore lint/correctness/useExhaustiveDependencies: take reads only refs and setters.
	useEffect(() => {
		if (models === null || !busy(models)) return;
		const timer = setTimeout(() => {
			void wire
				.command("voice.models", {})
				.then(take)
				.catch(() => {});
		}, POLL_MS);
		return () => clearTimeout(timer);
	}, [models]);

	const act = (cmd: ModelCommand, modelId: string) => {
		setRefusal(null);
		wire
			.command(cmd, { modelId })
			.then(take)
			.catch((error: Error) => setRefusal(error.message));
	};

	return { models, refusal, act };
}

/**
 * The desk's own speech models, the page under Settings › Voice: what is
 * installed and what it takes, then what Hotline offers to download, each
 * tagged with the job it does. Nothing is downloaded until the person
 * presses Download, whose label says how big it is; the desk checks what
 * arrives against the hash it pins (voice.md, Hearing on the desk).
 */
export function DeskModels({
	desk,
	options,
	voice,
}: {
	desk: DeskModelsState;
	/** What the jobs run on now, for each model's tag and whether calls use it; null from a desk too old to say. */
	options: CapabilityOptions | null;
	/** `settings.voice` as written, for whether calls run Turn by turn. */
	voice: unknown;
}) {
	const [credits, setCredits] = useState<string | null>(null);
	const { models, refusal, act } = desk;
	if (models === null) return refusal === null ? null : <Refusal message={refusal} />;
	const language = typeof navigator === "undefined" ? "en" : navigator.language;
	const installed = models.filter((model) => model.state === "installed");
	const offered = models.filter((model) => model.state !== "installed");

	const installedLine = (model: SpeechModel) =>
		model.error ??
		[
			modelLine(model),
			model.detail,
			options !== null && usedForCalls(model, options, voice) ? "Used for calls" : "",
			languageHint(model, models, language) ?? "",
		]
			.filter((part) => part !== "")
			.join(" · ");
	const offeredLine = (model: SpeechModel) =>
		model.error ?? [model.detail, modelLine(model)].filter((part) => part !== "").join(" · ");

	return (
		<>
			<p className="group-hint">{megabytes(usedBytes(models))} used</p>
			<section>
				<h3 className="group-title">Installed</h3>
				<div className="grouped">
					{installed.length === 0 ? (
						<p className="group-row text-sm text-ink-3">Nothing installed yet.</p>
					) : (
						installed.map((model) => (
							<Fragment key={model.id}>
								<div className="group-row">
									<ModelText model={model} kind={modelKind(model, options)} line={installedLine(model)} />
									<span className="flex shrink-0 items-center gap-1">
										{model.credit !== "" && (
											<button
												type="button"
												className="control btn-quiet btn-sm"
												aria-expanded={credits === model.id}
												aria-label={`Credits for ${model.name}`}
												onClick={() => setCredits((was) => (was === model.id ? null : model.id))}
											>
												Credits
											</button>
										)}
										<button type="button" className="control btn-quiet btn-sm" aria-label={`Remove ${model.name}`} onClick={() => act("voice.model_remove", model.id)}>
											Remove
										</button>
									</span>
								</div>
								{credits === model.id && (
									<div className="group-row use-for-nested group-row-detail" style={{ whiteSpace: "normal" }}>
										<span>
											{model.credit} (
											<a
												href={model.licenceUrl}
												onClick={(event) => {
													event.preventDefault();
													void openLink(model.licenceUrl);
												}}
											>
												licence
											</a>
											).
										</span>
									</div>
								)}
							</Fragment>
						))
					)}
				</div>
			</section>
			{offered.length > 0 && (
				<section>
					<h3 className="group-title">Recommended</h3>
					<div className="grouped">
						{offered.map((model) => (
							<div key={model.id} className="group-row">
								<ModelText model={model} kind={modelKind(model, options)} line={offeredLine(model)}>
									{model.state === "downloading" && (
										<span
											role="progressbar"
											aria-label={`Downloading ${model.name}`}
											aria-valuemin={0}
											aria-valuemax={100}
											aria-valuenow={Math.round(progress(model) * 100)}
											className="mt-1 block h-1 w-40 overflow-hidden rounded-full bg-line"
										>
											<span className="block h-full bg-accent" style={{ width: `${progress(model) * 100}%` }} />
										</span>
									)}
								</ModelText>
								<span className="flex shrink-0 items-center gap-1">
									{model.state === "available" && (
										<button type="button" className="control btn btn-sm" onClick={() => act("voice.model_install", model.id)}>
											Download {megabytes(model.downloadBytes)}
										</button>
									)}
									{(model.state === "downloading" || model.state === "unpacking") && (
										<button type="button" className="control btn-quiet btn-sm" aria-label={`Stop downloading ${model.name}`} onClick={() => act("voice.model_cancel", model.id)}>
											Cancel
										</button>
									)}
								</span>
							</div>
						))}
					</div>
				</section>
			)}
			{refusal !== null && <Refusal message={refusal} />}
		</>
	);
}

/** A model's name and the job it does, then the line under it. */
function ModelText({ model, kind, line, children }: { model: SpeechModel; kind: string; line: string; children?: React.ReactNode }) {
	return (
		<span className="group-row-text min-w-0">
			<span className="group-row-title">
				{model.name}
				<span className="provider-chip model-kind">{kind}</span>
			</span>
			{line !== "" && (
				<span className="group-row-detail" style={model.error === undefined ? undefined : { whiteSpace: "normal" }}>
					{line}
				</span>
			)}
			{children}
		</span>
	);
}
