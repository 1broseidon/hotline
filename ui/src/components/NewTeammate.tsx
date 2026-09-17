import { useEffect, useState } from "react";
import type { BackendChoice, ConfigChoice, PersonaDraft } from "../generated/contract";
import { chordKeys } from "../chords";
import { CloseIcon } from "../icons";
import { useRoomSettings } from "../room";
import { Band } from "../ui/Band";
import { Picker } from "../ui/Menu";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import { BackendPicker } from "./BackendPicker";
import { PathField } from "./PathField";
import { BACKGROUND_ABOUT, COMPUTER_ABOUT, MACHINE_ABOUT, SwitchRow } from "./Teammate";

/** Hotline Agent's stored backend id. Any other id is an ACP harness. */
const HOTLINE_AGENT = "hotline";

/**
 * Creating a teammate: the things the person decides, and nothing else.
 *
 * A teammate is an identity (`goal`), a workspace (`cwd`), a harness
 * (`backendId`) and — for Hotline Agent only — a disposition (`modelId`) under
 * a name. The harness defaults to the room's `defaultBackendId`. An ACP
 * harness brings its own models once the session is up, so that field is
 * not asked here. The access choices people most often decide up front —
 * the whole machine, background work, a computer — are asked too, with the
 * pane's own words; MCP and skill grants stay on the pane.
 *
 * Created, the teammate is started at once and opened — nobody adds a
 * colleague in order to look at them in a list.
 */
export function NewTeammate({
	models,
	onCreated,
	onClose,
}: {
	models: ConfigChoice[];
	onCreated(personaId: string): void;
	onClose(): void;
}) {
	return (
		<div className="pane">
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">New teammate</h2>
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			<Scroll>
				<NewTeammateForm className="pane-column" models={models} onCreated={onCreated} onCancel={onClose} />
			</Scroll>
		</div>
	);
}

/**
 * The form itself, without the pane around it: the new-teammate pane and
 * the welcome pane's second step both render this one, so the first
 * teammate is made the way every later one is. `goal` is a starting point
 * the person can keep or replace; the welcome pane suggests one, the pane
 * from the plus suggests nothing.
 */
export function NewTeammateForm({
	models,
	goal: suggestedGoal = "",
	submitLabel = "Add teammate",
	className,
	onCreated,
	onCancel,
}: {
	models: ConfigChoice[];
	goal?: string;
	submitLabel?: string;
	className?: string;
	onCreated(personaId: string): void;
	onCancel?: () => void;
}) {
	const { defaultBackendId, defaultModelId, lastModelId } = useRoomSettings();
	const [name, setName] = useState("");
	const [goal, setGoal] = useState(suggestedGoal);
	const [cwd, setCwd] = useState("");
	const [picked, setPicked] = useState<string | null>(null);
	const [backends, setBackends] = useState<BackendChoice[]>([]);
	const [pickedModel, setPickedModel] = useState<string | null>(null);
	const [machine, setMachine] = useState(false);
	const [backgroundWork, setBackgroundWork] = useState(false);
	const [computer, setComputer] = useState(false);
	// A computer is offered only where one can start.
	const [computerReady, setComputerReady] = useState(false);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const modelId = pickedModel ?? defaultModelId ?? lastModelId ?? "";
	// A blank draft runs on what the room prefers, else the first choice.
	const fallback = models.find((one) => one.id === (defaultModelId ?? lastModelId)) ?? models[0];

	useEffect(() => {
		void wire
			.command("backends.list", {})
			.then(setBackends)
			.catch((error: Error) => setRefusal(error.message));
		void wire
			.command("computer.runtimes", {})
			.then((reports) => setComputerReady(reports.some((one) => one.state === "ready")))
			.catch(() => setComputerReady(false));
	}, []);

	const available = (id: string) => backends.some((one) => one.id === id && one.unavailable === undefined);
	const backendId =
		picked ??
		(available(defaultBackendId)
			? defaultBackendId
			: (backends.find((one) => one.unavailable === undefined)?.id ?? ""));
	const onHotline = backendId === HOTLINE_AGENT || backendId === "";

	const submit = async () => {
		const trimmed = name.trim();
		if (!trimmed || busy) return;
		setBusy(true);
		setRefusal(null);
		const draft: PersonaDraft = { name: trimmed };
		if (goal.trim()) draft.goal = goal.trim();
		if (cwd.trim()) draft.cwd = cwd.trim();
		if (backendId) draft.backendId = backendId;
		if (onHotline && modelId) draft.modelId = modelId;
		if (onHotline && machine) draft.reach = "machine";
		if (backgroundWork) draft.backgroundWork = true;
		if (computerReady && computer) draft.computer = { enabled: true };
		try {
			const persona = await wire.command("persona.create", { draft });
			await wire.command("session.start", { personaId: persona.id });
			onCreated(persona.id);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	return (
		<form
			className={`${className ?? ""} flex flex-col gap-5`}
			onSubmit={(event) => {
				event.preventDefault();
				void submit();
			}}
		>
			<div>
				<label className="label" htmlFor="new-name">
					Name
				</label>
				<input
					id="new-name"
					className="field"
					value={name}
					autoFocus
					autoComplete="off"
					onChange={(event) => setName(event.target.value)}
				/>
			</div>

			<div>
				<label className="label" htmlFor="new-goal">
					Goal
				</label>
				<textarea
					id="new-goal"
					className="field"
					rows={3}
					placeholder="What this teammate is for."
					value={goal}
					onChange={(event) => setGoal(event.target.value)}
				/>
				<p className="hint">Written into the working directory as AGENTS.md, so the agent reads it on every start.</p>
			</div>

			<div>
				<label className="label" htmlFor="new-cwd">
					Working directory
				</label>
				<PathField id="new-cwd" value={cwd} placeholder="A folder under the data directory, unless you pick one" onChange={setCwd} />
			</div>

			{backends.length > 0 && (
				<div>
					<p className="label" id="new-backend">
						Runs on
					</p>
					<BackendPicker
						backends={backends}
						selected={backendId}
						name="new-backend"
						labelledBy="new-backend"
						onSelect={setPicked}
					/>
					{!onHotline && (
						<p className="hint">Permissions are managed by this external harness. Selecting it trusts its tools and configuration; Hotline's shell sandbox does not confine it.</p>
					)}
				</div>
			)}

			{onHotline && (
				<div>
					<p className="label" id="new-model">
						Model
					</p>
					<Picker
						field
						value={modelId}
						choices={[{ id: "", name: fallback === undefined ? "Whichever a key unlocks" : `${fallback.name} — the default` }, ...models]}
						placeholder="Model"
						label="Model"
						onChange={setPickedModel}
					/>
				</div>
			)}

			<div>
				<p className="label">Access</p>
				<div className="grouped">
					{onHotline && (
						<SwitchRow
							title="Whole machine"
							about={MACHINE_ABOUT}
							checked={machine}
							disabled={busy}
							onChange={setMachine}
						/>
					)}
					<SwitchRow
						title="Background work"
						about={BACKGROUND_ABOUT}
						checked={backgroundWork}
						disabled={busy}
						onChange={setBackgroundWork}
					/>
					{computerReady && (
						<SwitchRow title="Computer" about={COMPUTER_ABOUT} checked={computer} disabled={busy} onChange={setComputer} />
					)}
				</div>
				<p className="hint">These can always be changed later on the teammate's pane.</p>
			</div>

			{refusal !== null && (
				<p role="status" className="selectable text-sm text-danger">
					{refusal}
				</p>
			)}

			<div className="mt-1 flex justify-end gap-2">
				{onCancel !== undefined && (
					<button type="button" className="control btn" onClick={onCancel}>
						Cancel
					</button>
				)}
				<button type="submit" className="control btn-primary" disabled={busy || name.trim() === ""}>
					{busy ? "Setting up…" : submitLabel}
				</button>
			</div>
		</form>
	);
}
