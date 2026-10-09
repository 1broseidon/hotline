import { useEffect, useRef, useState } from "react";
import type { SpeechModel } from "../generated/contract";
import { openLink } from "../native";
import { Refusal } from "../ui/Refusal";
import { noteDeskModels } from "../voice/desk";
import { busy, installedChanged, megabytes, modelLine, progress } from "../voice/deskModels";
import { wire } from "../wire";

/** How often a download under way is asked after. */
const POLL_MS = 500;

/**
 * The desk's own speech models, one row each under Hearing: download one,
 * watch it come, cancel it, remove it. Nothing is downloaded until the
 * person presses Download, whose label says how big it is; the desk checks
 * what arrives against the hash it pins (voice.md, Hearing on the desk).
 */
export function DeskModels({ onInstalledChanged }: { onInstalledChanged(): void }) {
	const [models, setModels] = useState<SpeechModel[] | null>(null);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [credits, setCredits] = useState(false);
	const shown = useRef<SpeechModel[] | null>(null);
	const changed = useRef(onInstalledChanged);
	changed.current = onInstalledChanged;

	const take = (next: SpeechModel[]) => {
		noteDeskModels(next);
		if (shown.current !== null && installedChanged(shown.current, next)) changed.current();
		shown.current = next;
		setModels(next);
	};

	useEffect(() => {
		void wire
			.command("voice.models", {})
			.then(take)
			.catch((error: Error) => setRefusal(error.message));
	}, []);

	// A download is followed by asking again until it settles.
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

	const act = (cmd: "voice.model_install" | "voice.model_cancel" | "voice.model_remove", modelId: string) => {
		setRefusal(null);
		wire
			.command(cmd, { modelId })
			.then(take)
			.catch((error: Error) => setRefusal(error.message));
	};

	if (models === null || models.length === 0) return refusal === null ? null : <Refusal message={refusal} />;
	const credited = [...new Map(models.filter((model) => model.credit !== "").map((model) => [model.credit, model])).values()];
	return (
		<>
			{/* A title, not a paragraph: the list grows as models are added. */}
			<div className="group-row use-for-nested">
				<span className="group-row-text">
					<span className="group-row-title">Free and private local models</span>
				</span>
				{credited.length > 0 && (
					<button type="button" className="control btn-quiet btn-sm" aria-expanded={credits} onClick={() => setCredits((was) => !was)}>
						Credits
					</button>
				)}
			</div>
			{models.map((model) => (
				<div key={model.id} className="group-row use-for-nested">
					<span className="group-row-text min-w-0">
						<span className="group-row-title">{model.name}</span>
						{(model.error ?? modelLine(model)) !== "" && (
							<span className="group-row-detail" style={model.error === undefined ? undefined : { whiteSpace: "normal" }}>
								{model.error ?? modelLine(model)}
							</span>
						)}
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
					</span>
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
						{model.state === "installed" && (
							<button type="button" className="control btn-quiet btn-sm" aria-label={`Remove ${model.name}`} onClick={() => act("voice.model_remove", model.id)}>
								Remove
							</button>
						)}
					</span>
				</div>
			))}
			{credits && (
				<div className="group-row use-for-nested group-row-detail" style={{ whiteSpace: "normal" }}>
					<span>
						{credited.map((model) => (
							<span key={model.credit}>
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
								).{" "}
							</span>
						))}
					</span>
				</div>
			)}
			{refusal !== null && <Refusal message={refusal} />}
		</>
	);
}
