import { useEffect, useState } from "react";
import type { CapabilityOptions } from "../../generated/contract";
import { ChevronRightIcon } from "../../icons";
import { useModelsRevision, useRawSetting } from "../../room";
import { BackKey, Band } from "../../ui/Band";
import { Scroll } from "../../ui/Scroll";
import { installedSummary } from "../../voice/deskModels";
import { wire } from "../../wire";
import { DeskModels, useDeskModels } from "../DeskModels";
import { CallSettings } from "../UseFor";

/**
 * Voice: how calls run, then the desk's own speech models on a page under
 * it, since those are what Transcription and Speech run on when they run
 * on the desk. Providers keeps the accounts a call may also run on.
 *
 * The pane is this section's own rather than Settings', because it has a
 * page under it and the band has to say which one you are on, as
 * Providers, Tools and Skills do.
 */
export function VoiceSection({ onBack }: { onBack?: (() => void) | undefined }) {
	const [open, setOpen] = useState(false);
	/* What each job can run on. Null until it arrives, and for good on a desk too old to say. */
	const [capabilities, setCapabilities] = useState<CapabilityOptions | null>(null);
	const modelsRevision = useModelsRevision();
	const voice = useRawSetting("voice");

	const reloadCapabilities = () =>
		wire
			.command("capabilities.options", {})
			.then(setCapabilities)
			.catch(() => setCapabilities(null));

	// A connection or a model coming or going, or a call setting written, changes what it shows.
	// biome-ignore lint/correctness/useExhaustiveDependencies: voice and modelsRevision are what change the answer; reloadCapabilities reads only a setter.
	useEffect(() => {
		void reloadCapabilities();
	}, [modelsRevision, JSON.stringify(voice)]);

	// A model installed or removed changes what can hear and speak.
	const desk = useDeskModels(() => void reloadCapabilities());

	if (open) {
		return (
			<div className="pane">
				<Band>
					<BackKey onBack={() => setOpen(false)} />
					<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Voice models</h2>
				</Band>
				<Scroll>
					<div className="pane-column flex flex-col gap-6">
						<DeskModels desk={desk} options={capabilities} voice={voice} />
					</div>
				</Scroll>
			</div>
		);
	}

	return (
		<div className="pane">
			<Band>
				{onBack !== undefined && <BackKey onBack={onBack} />}
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Voice</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					{capabilities !== null && <CallSettings options={capabilities} voice={voice} onChanged={() => void reloadCapabilities()} />}
					<section>
						<div className="grouped">
							<button type="button" className="group-row group-row-choice group-row-page w-full text-left" onClick={() => setOpen(true)}>
								<span className="group-row-text">
									<span className="group-row-title">Voice models</span>
								</span>
								{desk.models !== null && <span className="shrink-0 text-sm text-ink-3">{installedSummary(desk.models)}</span>}
								<ChevronRightIcon className="shrink-0 text-ink-3" />
							</button>
						</div>
					</section>
				</div>
			</Scroll>
		</div>
	);
}
