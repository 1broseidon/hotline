import { useEffect } from "react";
import { plainModel } from "../accounts";
import type { ConfigChoice, SessionConfig } from "../generated/contract";
import { lastKnown, rememberKnown } from "../lastKnown";
import { refreshRoomRead, useRoomRead } from "../roomReads";
import { useRoomSettings } from "../room";
import { Picker } from "../ui/Menu";
import { type MenuTeammate, ModelPicker } from "./ModelPicker";
import { wire, type RosterEntry } from "../wire";

/** Hotline Agent's stored backend id. Any other id is an ACP harness. */
const HOTLINE_AGENT = "hotline";

/**
 * The open teammate's model and effort, on the window's top strip, in the
 * pill that is the teammate's menu too. They name what a turn would run on
 * whether or not a session is up, and offer the same choices: a harness
 * teammate at rest has what its agent last offered on this desk, filled in
 * by the core, with its own choices current, so nothing goes missing
 * because the agent is not running. Only an agent that has never reported
 * here is sparse: the pill names the harness. Mode is not here: it is the
 * teammate's own and lives in the inspector.
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
	onAddAccount,
	teammate,
	compact = false,
}: {
	entry: RosterEntry;
	models: ConfigChoice[];
	onSaid(said: string | null): void;
	/** Settings › Providers, where accounts are added. */
	onAddAccount?: (() => void) | undefined;
	/** The teammate whose menu the pill is too: face, name, goal and actions. */
	teammate?: MenuTeammate | undefined;
	/** A narrow band: the pill is the model's mark and the face. */
	compact?: boolean;
}) {
	const { persona, session } = entry;
	const personaId = persona.id;
	const { defaultModelId, lastModelId } = useRoomSettings();

	const hotline = persona.backendId === HOTLINE_AGENT;
	const modelChoices = session.models.length > 0 ? session.models : hotline ? models : [];
	// The strip names the model a turn would run on, whether or not a
	// session is up. For Hotline Agent that is the driver's own rule: the
	// teammate's choice when the list still has it, else the room default,
	// else the last model used, else the first choice — newest only on a
	// desk that has never run a model. For a harness it is the session's,
	// which at rest the core fills with the teammate's choice or its agent's
	// default; before the agent has ever reported here, the harness's own
	// name stands where the model will.
	const currentModel =
		session.currentModelId ??
		(hotline ? hotlineModel(persona.modelId, defaultModelId, lastModelId, modelChoices) : (persona.modelId ?? ""));
	// Every agent the teammate could run on, for the picker's Agents (task-287).
	const backends = useRoomRead("backends.list", "", true);
	const harnessName = backends?.find((one) => one.id === persona.backendId)?.name ?? null;
	const idleEfforts = useRoomRead("models.efforts", currentModel, hotline && currentModel !== "");
	const configs: SessionConfig[] =
		session.configs.length > 0
			? session.configs
			: hotline && idleEfforts !== undefined && idleEfforts.choices.length > 0
				? [
						{
							id: "effort",
							name: "Effort",
							category: "effort",
							// The stored effort, else the one the session will run at.
							currentId: persona.effortId ?? idleEfforts.defaultId ?? "",
							options: idleEfforts.choices,
						},
					]
				: [];
	const setModel = async (modelId: string) => {
		onSaid(null);
		await wire.command("session.set_model", { personaId, modelId }).catch((error: Error) => onSaid(error.message));
	};
	const setConfig = async (configId: string, value: string) => {
		onSaid(null);
		await wire.command("session.set_config", { personaId, configId, value }).catch((error: Error) => onSaid(error.message));
	};

	const setBackend = async (backendId: string, modelId: string | undefined) => {
		onSaid(null);
		await wire
			.command("session.set_backend", { personaId, backendId, ...(modelId !== undefined ? { modelId } : {}) })
			.catch((error: Error) => onSaid(error.message));
	};
	const agents = (backends ?? []).map((backend) => ({
		id: backend.id,
		name: backend.name,
		...(backend.unavailable !== undefined && backend.unavailable !== null ? { unavailable: backend.unavailable } : {}),
		...(backend.signIn === true ? { signIn: true } : {}),
		// The live session's list, else the one the agent last reported here.
		choices: backend.id === persona.backendId && session.models.length > 0 ? session.models : (backend.models ?? []),
	}));
	// Before the agents are read, the teammate's own still lists what its
	// session offers, live or as its agent last offered it.
	if (!hotline && !agents.some((agent) => agent.id === persona.backendId)) {
		agents.push({ id: persona.backendId, name: harnessName ?? persona.backendId, choices: session.models });
	}

	// The model's pretty name, from any list the window holds, and the effort
	// a live session runs at. What a live session names is remembered, so a
	// resting teammate's strip still says it: never a model id, never a
	// model without the effort it last ran at.
	const listed = [...session.models, ...(agents.find((agent) => agent.id === persona.backendId)?.choices ?? []), ...models];
	const namedHere = listed.find((choice) => choice.id === currentModel || plainModel(choice.id) === plainModel(currentModel))?.name;
	const liveEffort = session.configs.find((config) => config.category === "effort");
	const liveEffortName = liveEffort?.options.find((option) => option.id === liveEffort.currentId)?.name ?? null;
	const live = session.currentModelId !== undefined;
	useEffect(() => {
		if (live && currentModel !== "" && namedHere !== undefined) {
			rememberKnown(personaId, { modelId: currentModel, name: namedHere, effort: liveEffortName });
		}
	}, [live, personaId, currentModel, namedHere, liveEffortName]);
	const known = lastKnown(personaId, currentModel);

	// Model, account, effort and agent in one pull-down: Hotline Agent's
	// accounts, then the other agents. It is always there, being the
	// teammate's menu as well: before the agents are read it has the model
	// alone, and a Hotline Agent teammate with no account left can still
	// move to an installed agent or add an account.
	const effort = configs.find((config) => config.category === "effort");
	// A harness that has never reported its levels here offers none to
	// change: the strip names the one it last ran at, a single level, so
	// nothing opens for it.
	const shownEffort =
		effort !== undefined
			? { value: effort.currentId ?? "", choices: effort.options }
			: known?.effort != null
				? { value: known.effort, choices: [{ id: known.effort, name: known.effort }] }
				: null;
	const others = configs.filter((config) => config.category !== "effort");
	return (
		<>
			{others.map((config) => (
				<Picker
					key={config.id}
					value={config.currentId ?? ""}
					choices={config.options}
					placeholder={config.name}
					label={config.name}
					onChange={(value) => void setConfig(config.id, value)}
				/>
			))}
			<ModelPicker
				choices={hotline ? modelChoices : models}
				modelId={currentModel}
				backendId={persona.backendId}
				agents={agents}
				onAgent={setBackend}
				onOpen={() => {
					refreshRoomRead("backends.list");
					// The plan under the model is the account's reading; the
					// core reads it again only when it is a few minutes old.
					void wire.command("plan.refresh", { personaId }).catch(() => {});
				}}
				{...(!hotline && currentModel === "" && harnessName !== null ? { fallbackName: harnessName } : {})}
				{...(known !== null ? { knownName: known.name } : {})}
				effort={shownEffort}
				label={session.modelLabel ?? "Model"}
				plan={session.planUsage}
				onModel={setModel}
				onEffort={(value) => setConfig(effort?.id ?? "effort", value)}
				onAddAccount={onAddAccount}
				teammate={teammate}
				compact={compact}
			/>
		</>
	);
}

function hotlineModel(
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
