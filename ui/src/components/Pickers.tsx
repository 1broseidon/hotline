import { useEffect, useState } from "react";
import type { ConfigChoice, SessionConfig } from "../generated/contract";
import { useRoomSettings } from "../room";
import { Picker } from "../ui/Menu";
import { wire, type RosterEntry } from "../wire";

/** Toad Agent's stored backend id. Any other id is an ACP harness. */
const TOAD_AGENT = "pi";

/**
 * The open teammate's model and effort, on the window's top strip. They
 * name what a turn would run on whether or not a session is up, so the
 * strip is never blank over a resting teammate. Mode is not here: it is
 * the teammate's own and lives in the inspector.
 *
 * A refusal from the core is handed up, not shown: the strip has no room
 * for a sentence, and the conversation says it under its band. Keyed by
 * teammate above, so one teammate's harness name never stands in for
 * another's model.
 */
export function SessionPickers({
	entry,
	models,
	onSaid,
}: {
	entry: RosterEntry;
	models: ConfigChoice[];
	onSaid(said: string | null): void;
}) {
	const { persona, session } = entry;
	const personaId = persona.id;
	const { defaultModelId, lastModelId } = useRoomSettings();
	const [idleEfforts, setIdleEfforts] = useState<ConfigChoice[]>([]);

	const toad = persona.backendId === TOAD_AGENT;
	const modelChoices = session.models.length > 0 ? session.models : toad ? models : [];
	// The strip names the model a turn would run on, whether or not a
	// session is up. For Toad Agent that is the driver's own rule: the
	// teammate's choice when the list still has it, else the room default,
	// else the last model used, else the first choice — newest only on a
	// desk that has never run a model. For a harness it is the last model a
	// session reported, remembered on the teammate; before any ever has,
	// the harness's own name stands where the model will.
	const currentModel =
		session.currentModelId ??
		(toad ? toadModel(persona.modelId, defaultModelId, lastModelId, modelChoices) : (persona.modelId ?? ""));
	const [harnessName, setHarnessName] = useState<string | null>(null);
	useEffect(() => {
		if (toad || currentModel !== "") return;
		let cancelled = false;
		void wire.command("backends.list", {}).then(
			(list) => {
				if (!cancelled) setHarnessName(list.find((one) => one.id === persona.backendId)?.name ?? null);
			},
			() => {
				if (!cancelled) setHarnessName(null);
			},
		);
		return () => {
			cancelled = true;
		};
	}, [toad, currentModel, persona.backendId]);
	const restingModel = currentModel !== "" ? currentModel : harnessName;
	// An idle Toad Agent session carries no configs. The strip derives the
	// effort picker the same way it derives currentModel: the catalogue
	// for the model a turn would run on.
	useEffect(() => {
		if (!toad || currentModel === "") {
			setIdleEfforts([]);
			return;
		}
		let cancelled = false;
		void wire.command("models.efforts", { modelId: currentModel }).then(
			(choices) => {
				if (!cancelled) setIdleEfforts(choices);
			},
			() => {
				if (!cancelled) setIdleEfforts([]);
			},
		);
		return () => {
			cancelled = true;
		};
	}, [toad, currentModel]);
	const configs: SessionConfig[] =
		session.configs.length > 0
			? session.configs
			: toad && idleEfforts.length > 0
				? [{ id: "effort", name: "Effort", category: "effort", currentId: persona.effortId ?? "", options: idleEfforts }]
				: [];
	const visibleConfigs = toad ? configs : configs.filter((config) => config.category === "effort");

	return (
		<>
			{modelChoices.length > 0 ? (
				<Picker
					value={currentModel}
					choices={modelChoices}
					placeholder="Model"
					label={session.modelLabel ?? "Model"}
					onChange={(modelId) => {
						onSaid(null);
						void wire
							.command("session.set_model", { personaId, modelId })
							.catch((error: Error) => onSaid(error.message));
					}}
				/>
			) : (
				restingModel !== null && (
					<span
						className="control max-w-[220px] truncate px-2 font-medium text-ink-2"
						title={currentModel !== "" ? "The model of the last session; the list comes once it starts" : "Starts on its own model"}
					>
						{restingModel}
					</span>
				)
			)}
			{visibleConfigs.map((config) => (
				<Picker
					key={config.id}
					value={config.currentId ?? ""}
					choices={config.options}
					placeholder={config.name}
					label={config.name}
					onChange={(value) => {
						onSaid(null);
						void wire
							.command("session.set_config", { personaId, configId: config.id, value })
							.catch((error: Error) => onSaid(error.message));
					}}
				/>
			))}
		</>
	);
}

function toadModel(
	chosen: string | undefined,
	defaultModelId: string | null,
	lastModelId: string | null,
	choices: ConfigChoice[],
): string {
	if (chosen !== undefined && choices.some((one) => one.id === chosen)) return chosen;
	if (defaultModelId !== null && choices.some((one) => one.id === defaultModelId)) return defaultModelId;
	if (lastModelId !== null && choices.some((one) => one.id === lastModelId)) return lastModelId;
	return choices[0]?.id ?? "";
}
