import type { ConfigChoice } from "../generated/contract";
import type { RosterEntry } from "../wire";

/**
 * Who you are talking to, what their session is doing, and the one thing you
 * change mid-conversation often enough to deserve the header: the model.
 *
 * The session's own list wins when it has one, because a running agent knows
 * what it can actually be switched to; the room's list is what a teammate that
 * has not started yet can be pointed at.
 */
export function ChatHeader({
	entry,
	models,
	onSetModel,
	onOpenKeys,
}: {
	entry: RosterEntry;
	models: ConfigChoice[];
	onSetModel(modelId: string): void;
	onOpenKeys(): void;
}) {
	const { persona, session } = entry;
	const choices = session.models.length > 0 ? session.models : models;
	const current = session.currentModelId ?? persona.modelId ?? "";

	return (
		<header className="flex items-center gap-3 border-b border-rule bg-paper px-6 py-2.5">
			<div className="min-w-0">
				<h2 className="truncate font-medium">{persona.name}</h2>
				<p className="truncate font-mono text-xs text-ink-3">{persona.cwd}</p>
			</div>

			<p className="ml-auto shrink-0 text-xs text-ink-3">
				{session.error ?? session.state}
			</p>

			<select
				className="field w-auto max-w-56 shrink-0 text-xs"
				aria-label="Model"
				value={choices.some((one) => one.id === current) ? current : ""}
				onChange={(event) => onSetModel(event.target.value)}
			>
				{/* A model the session reports but the list has not named yet still
				    has to be selectable-looking, or the picker would silently claim
				    the teammate is on something else. */}
				<option value="" disabled>
					{current || "Model"}
				</option>
				{choices.map((model) => (
					<option key={model.id} value={model.id}>
						{model.name}
					</option>
				))}
			</select>

			<button type="button" className="btn-quiet shrink-0" title="Keys (Ctrl+,)" onClick={onOpenKeys}>
				Keys
			</button>
		</header>
	);
}
