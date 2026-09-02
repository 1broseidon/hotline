import type { ConfigChoice } from "../generated/contract";
import type { RosterEntry } from "../wire";

/**
 * Who you are talking to, what their session is doing, the model, and the
 * doors that belong to this conversation: search, a new chapter, and the
 * teammate itself.
 *
 * The session's own list wins when it has one, because a running agent knows
 * what it can actually be switched to; the room's list is what a teammate that
 * has not started yet can be pointed at. The model picker stays here — it is
 * disposition, not identity, and identity lives on the teammate sheet.
 */
export function ChatHeader({
	entry,
	models,
	searchOpen,
	chapterSaid,
	onSetModel,
	onOpenTeammate,
	onOpenSearch,
	onNewChapter,
}: {
	entry: RosterEntry;
	models: ConfigChoice[];
	searchOpen: boolean;
	chapterSaid: string | null;
	onSetModel(modelId: string): void;
	onOpenTeammate(): void;
	onOpenSearch(): void;
	onNewChapter(): void;
}) {
	const { persona, session } = entry;
	const choices = session.models.length > 0 ? session.models : models;
	const current = session.currentModelId ?? persona.modelId ?? "";

	return (
		<header className="flex items-center gap-3 border-b border-rule bg-paper px-6 py-2.5">
			<button
				type="button"
				className="min-w-0 text-left"
				title="Teammate"
				aria-label={`${persona.name}'s settings`}
				onClick={onOpenTeammate}
			>
				<h2 className="truncate font-medium">{persona.name}</h2>
				<p className="truncate font-mono text-xs text-ink-3">{persona.cwd}</p>
			</button>

			<p className="ml-auto shrink-0 text-xs text-ink-3">{session.error ?? session.state}</p>

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

			<button
				type="button"
				className={`btn-quiet shrink-0 ${searchOpen ? "bg-paper-3" : ""}`}
				title="Search (Ctrl+F)"
				aria-expanded={searchOpen}
				onClick={onOpenSearch}
			>
				Search
			</button>

			<button
				type="button"
				className="btn-quiet shrink-0"
				title="Close this chapter and start the next one fresh"
				onClick={onNewChapter}
			>
				New chapter
			</button>

			<button
				type="button"
				className="btn-quiet shrink-0"
				title="Teammate (Ctrl+I)"
				aria-label="Teammate"
				onClick={onOpenTeammate}
			>
				…
			</button>

			{chapterSaid !== null && (
				<p className="shrink-0 text-xs text-[var(--danger)]">{chapterSaid}</p>
			)}
		</header>
	);
}
