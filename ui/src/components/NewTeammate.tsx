import { useState } from "react";
import type { ConfigChoice, PersonaDraft } from "../generated/contract";
import { wire } from "../wire";
import { Sheet } from "./Sheet";

/**
 * Creating a teammate: the four things the person decides, and nothing else.
 *
 * A teammate is an identity (`goal`), a workspace (`cwd`) and a disposition
 * (`modelId`) under a name. Everything else the room supplies, so everything
 * else is left off the form.
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
	const [modelId, setModelId] = useState("");
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	const submit = async () => {
		const trimmed = name.trim();
		if (!trimmed || busy) return;
		setBusy(true);
		setRefusal(null);
		const draft: PersonaDraft = { name: trimmed };
		if (goal.trim()) draft.goal = goal.trim();
		if (cwd.trim()) draft.cwd = cwd.trim();
		if (modelId) draft.modelId = modelId;
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
