import type { ConfigChoice } from "../generated/contract";
import type { RosterEntry } from "../wire";

/** Toad Agent's stored backend id. Any other id is an ACP harness. */
const TOAD_AGENT = "pi";

/**
 * Who you are talking to, what their session is doing, the model, the mode,
 * and the doors that belong to this conversation: search, a new chapter, and
 * the teammate itself.
 *
 * The session's own list wins when it has one, because a running agent knows
 * what it can actually be switched to. Toad Agent can still be pointed at the
 * room's list before it starts; an ACP harness has no models until the session
 * reports them, so that picker stays off rather than offering the desk's.
 * A failed start keeps its sentence here — the rail's dot already says error.
 */
export function ChatHeader({
	entry,
	models,
	searchOpen,
	chapterBusy,
	chapterSaid,
	onSetModel,
	onSetMode,
	onOpenTeammate,
	onOpenSearch,
	onNewChapter,
}: {
	entry: RosterEntry;
	models: ConfigChoice[];
	searchOpen: boolean;
	chapterBusy: boolean;
	chapterSaid: string | null;
	onSetModel(modelId: string): void;
	onSetMode(modeId: string): void;
	onOpenTeammate(): void;
	onOpenSearch(): void;
	onNewChapter(): void;
}) {
	const { persona, session } = entry;
	const toad = persona.backendId === TOAD_AGENT;
	const choices = session.models.length > 0 ? session.models : toad ? models : [];
	const current = session.currentModelId ?? persona.modelId ?? "";
	const showModel = choices.length > 0 || (toad && current !== "");
	const modes = session.modes;
	const currentMode = session.currentModeId ?? persona.modeId ?? "";

	return (
		<header className="border-b border-rule bg-paper">
			<div className="flex items-center gap-3 px-6 py-2.5">
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

				<p className="ml-auto shrink-0 text-xs text-ink-3">{session.state}</p>

				{showModel && (
					<select
						className="field w-auto max-w-56 shrink-0 text-xs"
						aria-label={session.modelLabel ?? "Model"}
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
				)}

				{modes.length > 0 && (
					<select
						className="field w-auto max-w-56 shrink-0 text-xs"
						aria-label={session.modeLabel ?? "Mode"}
						value={modes.some((one) => one.id === currentMode) ? currentMode : ""}
						onChange={(event) => onSetMode(event.target.value)}
					>
						<option value="" disabled>
							{currentMode || session.modeLabel || "Mode"}
						</option>
						{modes.map((mode) => (
							<option key={mode.id} value={mode.id}>
								{mode.name}
							</option>
						))}
					</select>
				)}

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
					disabled={chapterBusy}
					aria-busy={chapterBusy}
					onClick={onNewChapter}
				>
					New chapter
				</button>

				{chapterSaid !== null && (
					<p role="status" className="min-w-0 shrink text-xs text-[var(--danger)]">
						{chapterSaid}
					</p>
				)}

				<button
					type="button"
					className="btn-quiet shrink-0"
					title="Teammate (Ctrl+I)"
					aria-label="Teammate"
					onClick={onOpenTeammate}
				>
					…
				</button>
			</div>
			{session.error !== undefined && session.error !== "" && (
				<p role="status" className="px-6 pb-2.5 text-xs text-[var(--danger)]">
					{session.error}
				</p>
			)}
		</header>
	);
}
