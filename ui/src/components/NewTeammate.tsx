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

/** Toad Agent's stored backend id. Any other id is an ACP harness. */
const TOAD_AGENT = "pi";

/**
 * Creating a teammate: the things the person decides, and nothing else.
 *
 * A teammate is an identity (`goal`), a workspace (`cwd`), a harness
 * (`backendId`) and — for Toad Agent only — a disposition (`modelId`) under
 * a name. The harness defaults to the room's `defaultBackendId`. An ACP
 * harness brings its own models once the session is up, so that field is
 * not asked here.
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
	const { defaultBackendId } = useRoomSettings();
	const [name, setName] = useState("");
	const [goal, setGoal] = useState("");
	const [cwd, setCwd] = useState("");
	const [picked, setPicked] = useState<string | null>(null);
	const [backends, setBackends] = useState<BackendChoice[]>([]);
	const [modelId, setModelId] = useState("");
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	useEffect(() => {
		void wire
			.command("backends.list", {})
			.then(setBackends)
			.catch((error: Error) => setRefusal(error.message));
	}, []);

	const available = (id: string) => backends.some((one) => one.id === id && one.unavailable === undefined);
	const backendId =
		picked ??
		(available(defaultBackendId)
			? defaultBackendId
			: (backends.find((one) => one.unavailable === undefined)?.id ?? ""));
	const onToad = backendId === TOAD_AGENT || backendId === "";

	const submit = async () => {
		const trimmed = name.trim();
		if (!trimmed || busy) return;
		setBusy(true);
		setRefusal(null);
		const draft: PersonaDraft = { name: trimmed };
		if (goal.trim()) draft.goal = goal.trim();
		if (cwd.trim()) draft.cwd = cwd.trim();
		if (backendId) draft.backendId = backendId;
		if (onToad && modelId) draft.modelId = modelId;
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
		<div className="pane">
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">New teammate</h2>
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			<Scroll>
				<form
					className="pane-column flex flex-col gap-5"
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
						</div>
					)}

					{onToad && (
						<div>
							<p className="label" id="new-model">
								Model
							</p>
							<Picker
								field
								value={modelId}
								choices={[{ id: "", name: models[0] === undefined ? "Whichever a key unlocks" : `${models[0].name} — the default` }, ...models]}
								placeholder="Model"
								label="Model"
								onChange={setModelId}
							/>
						</div>
					)}

					{refusal !== null && (
						<p role="status" className="selectable text-sm text-danger">
							{refusal}
						</p>
					)}

					<div className="mt-1 flex justify-end gap-2">
						<button type="button" className="control btn" onClick={onClose}>
							Cancel
						</button>
						<button type="submit" className="control btn-primary" disabled={busy || name.trim() === ""}>
							{busy ? "Setting up…" : "Add teammate"}
						</button>
					</div>
				</form>
			</Scroll>
		</div>
	);
}
