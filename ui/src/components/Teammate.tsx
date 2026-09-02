import { useEffect, useState } from "react";
import type { Persona } from "../generated/contract";
import { wire } from "../wire";
import { Sheet } from "./Sheet";

/**
 * Editing a teammate: the four things the person decides, and the way out.
 *
 * Name, goal and working directory are the identity and the wall. Reach is
 * the one policy — the working directory, or the whole machine — and it is a
 * toggle because those are the only two answers. Deleting asks for the name
 * typed back, so a misfire in a sheet that also has Escape as its door does
 * not take a colleague with it.
 */
export function Teammate({
	persona,
	onClose,
	onDeleted,
}: {
	persona: Persona;
	onClose(): void;
	onDeleted(): void;
}) {
	const [name, setName] = useState(persona.name);
	const [goal, setGoal] = useState(persona.goal);
	const [cwd, setCwd] = useState(persona.cwd);
	const [confirm, setConfirm] = useState("");
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	useEffect(() => {
		setName(persona.name);
		setGoal(persona.goal);
		setCwd(persona.cwd);
	}, [persona.name, persona.goal, persona.cwd]);

	const save = (patch: Partial<Persona>) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		void wire
			.command("persona.update", { id: persona.id, patch })
			.catch((error: Error) => setRefusal(error.message))
			.finally(() => setBusy(false));
	};

	const saveName = () => {
		const trimmed = name.trim();
		if (!trimmed || trimmed === persona.name) {
			setName(persona.name);
			return;
		}
		save({ name: trimmed });
	};

	const saveGoal = () => {
		if (goal === persona.goal) return;
		save({ goal });
	};

	const saveCwd = () => {
		const trimmed = cwd.trim();
		if (!trimmed || trimmed === persona.cwd) {
			setCwd(persona.cwd);
			return;
		}
		save({ cwd: trimmed });
	};

	const remove = async () => {
		if (confirm !== persona.name || busy) return;
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("persona.delete", { id: persona.id });
			onDeleted();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	const machine = persona.reach === "machine";

	return (
		<Sheet title={persona.name} onClose={onClose}>
			<form
				className="flex flex-col gap-3"
				onSubmit={(event) => {
					event.preventDefault();
					saveName();
					saveGoal();
					saveCwd();
				}}
			>
				<div>
					<label className="label" htmlFor="edit-name">
						Name
					</label>
					<input
						id="edit-name"
						className="field"
						value={name}
						autoFocus
						onChange={(event) => setName(event.target.value)}
						onBlur={saveName}
					/>
				</div>

				<div>
					<label className="label" htmlFor="edit-goal">
						Goal
					</label>
					<textarea
						id="edit-goal"
						className="field resize-none"
						rows={3}
						placeholder="What this teammate is for."
						value={goal}
						onChange={(event) => setGoal(event.target.value)}
						onBlur={saveGoal}
					/>
				</div>

				<div>
					<label className="label" htmlFor="edit-cwd">
						Working directory
					</label>
					<input
						id="edit-cwd"
						className="field font-mono text-xs"
						spellCheck={false}
						value={cwd}
						onChange={(event) => setCwd(event.target.value)}
						onBlur={saveCwd}
					/>
				</div>

				<div>
					<p className="label">Reach</p>
					<label className="flex items-center gap-2 text-sm text-ink-2">
						<input
							type="checkbox"
							checked={machine}
							disabled={busy}
							onChange={(event) =>
								// A missing key leaves the old reach. The generated
								// patch is Partial<Persona>, so the wall is the
								// word, not JSON null.
								save({ reach: event.target.checked ? "machine" : "workspace" })
							}
						/>
						Whole machine
					</label>
					<p className="mt-1 text-xs leading-relaxed text-ink-3">
						{machine
							? "Tools can touch the rest of the machine. The working directory is where they start, not a wall."
							: "Tools stop at the working directory: nothing outside it can be read, changed, or run."}
					</p>
				</div>

				<section className="mt-2 border-t border-rule pt-4">
					<p className="label">Remove teammate</p>
					<p className="mb-2 text-xs leading-relaxed text-ink-3">
						Type <span className="font-medium text-ink-2">{persona.name}</span> to confirm. Their
						conversation goes too.
					</p>
					<input
						id="edit-confirm"
						className="field"
						aria-label="Type the teammate's name to confirm removal"
						value={confirm}
						onChange={(event) => setConfirm(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter") {
								event.preventDefault();
								void remove();
							}
						}}
					/>
					<div className="mt-3 flex justify-end">
						<button
							type="button"
							className="btn-quiet text-[var(--danger)]"
							disabled={busy || confirm !== persona.name}
							onClick={() => void remove()}
						>
							Remove teammate
						</button>
					</div>
				</section>

				{refusal !== null && <p className="text-xs text-[var(--danger)]">{refusal}</p>}

				<div className="mt-1 flex justify-end">
					<button type="button" className="btn-quiet" onClick={onClose}>
						Done
					</button>
				</div>
			</form>
		</Sheet>
	);
}
