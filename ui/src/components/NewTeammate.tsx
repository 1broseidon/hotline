import { useEffect, useState } from "react";
import type { BackendChoice, ConfigChoice, PersonaDraft } from "../generated/contract";
import { wire } from "../wire";
import { Sheet } from "./Sheet";

/** Toad Agent's stored backend id. Any other id is an ACP harness. */
const TOAD_AGENT = "pi";

/**
 * Creating a teammate: the things the person decides, and nothing else.
 *
 * A teammate is an identity (`goal`), a workspace (`cwd`), a harness
 * (`backendId`) and — for Toad Agent only — a disposition (`modelId`) under
 * a name. An ACP harness brings its own models once the session is up, so
 * that field is not asked here.
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
	const [name, setName] = useState("");
	const [goal, setGoal] = useState("");
	const [cwd, setCwd] = useState("");
	const [backendId, setBackendId] = useState("");
	const [backends, setBackends] = useState<BackendChoice[]>([]);
	const [modelId, setModelId] = useState("");
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	useEffect(() => {
		void wire
			.command("backends.list", {})
			.then((list) => {
				setBackends(list);
				setBackendId((current) => {
					if (current && list.some((one) => one.id === current && one.unavailable === undefined)) {
						return current;
					}
					return list.find((one) => one.unavailable === undefined)?.id ?? "";
				});
			})
			.catch((error: Error) => setRefusal(error.message));
	}, []);

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
		<Sheet title="New teammate" onClose={onClose}>
			<form
				className="flex flex-col gap-3"
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
						onChange={(event) => setName(event.target.value)}
					/>
				</div>

				<div>
					<label className="label" htmlFor="new-goal">
						Goal
					</label>
					<textarea
						id="new-goal"
						className="field resize-none"
						rows={3}
						placeholder="What this teammate is for."
						value={goal}
						onChange={(event) => setGoal(event.target.value)}
					/>
				</div>

				<div>
					<label className="label" htmlFor="new-cwd">
						Working directory
					</label>
					{/* Typed, not picked: the window has no file dialog of its own
					    and a path is a thing people already know how to write. */}
					<input
						id="new-cwd"
						className="field font-mono text-xs"
						placeholder="/home/you/projects/thing"
						spellCheck={false}
						value={cwd}
						onChange={(event) => setCwd(event.target.value)}
					/>
				</div>

				{backends.length > 0 && (
					<div>
						<p className="label" id="new-backend">
							Runs on
						</p>
						<div
							role="radiogroup"
							aria-labelledby="new-backend"
							className="flex flex-col gap-1.5"
						>
							{backends.map((backend) => (
								<BackendRow
									key={backend.id}
									backend={backend}
									selected={backendId === backend.id}
									onSelect={() => setBackendId(backend.id)}
								/>
							))}
						</div>
					</div>
				)}

				{onToad && (
					<div>
						<label className="label" htmlFor="new-model">
							Model
						</label>
						<select
							id="new-model"
							className="field"
							value={modelId}
							onChange={(event) => setModelId(event.target.value)}
						>
							<option value="">The room's default</option>
							{models.map((model) => (
								<option key={model.id} value={model.id}>
									{model.group ? `${model.group} · ${model.name}` : model.name}
								</option>
							))}
						</select>
					</div>
				)}

				{refusal !== null && <p className="text-xs text-[var(--danger)]">{refusal}</p>}

				<div className="mt-1 flex justify-end gap-2">
					<button type="button" className="btn-quiet" onClick={onClose}>
						Never mind
					</button>
					<button type="submit" className="btn-primary" disabled={busy || name.trim() === ""}>
						{busy ? "Setting up…" : "Add teammate"}
					</button>
				</div>
			</form>
		</Sheet>
	);
}

/**
 * One harness the room can name. An unavailable row stays in the list so
 * the missing piece is a sentence next to the name, not a hole.
 */
function BackendRow({
	backend,
	selected,
	onSelect,
}: {
	backend: BackendChoice;
	selected: boolean;
	onSelect(): void;
}) {
	const missing = backend.unavailable;
	return (
		<label
			className={`flex items-start gap-2 rounded-lg px-2.5 py-2 text-sm ${
				missing ? "bg-paper text-ink-3 opacity-60" : "bg-paper-3 text-ink-2"
			}`}
		>
			<input
				type="radio"
				name="new-backend"
				className="mt-0.5"
				checked={selected}
				disabled={missing !== undefined}
				onChange={onSelect}
			/>
			<span className="min-w-0 flex-1">
				<span className={`font-medium ${missing ? "text-ink-3" : "text-ink"}`}>{backend.name}</span>
				{backend.description !== "" && (
					<span className="mt-0.5 block text-xs leading-relaxed text-ink-3">
						{backend.description}
					</span>
				)}
				{missing !== undefined && (
					<span className="mt-0.5 block text-xs leading-relaxed">{missing}</span>
				)}
			</span>
		</label>
	);
}
