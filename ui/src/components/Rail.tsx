import { chordKeys } from "../chords";
import { GearIcon, MoreIcon, PlusIcon } from "../icons";
import { popupTeammateMenu } from "../native";
import type { SessionState } from "../generated/contract";
import type { Connection, RosterEntry } from "../wire";
import { Avatar } from "../ui/Avatar";
import { Band } from "../ui/Band";
import { MenuButton, type MenuEntry } from "../ui/Menu";

/**
 * Each teammate carries a vital sign rather than a status pill: the rail is
 * a roster you watch, so the one moving thing in the whole window is
 * whichever agent is currently working. A teammate at rest has no mark and
 * no label, whether or not a session is up behind them — that is plumbing,
 * and they are there either way.
 */
const VITAL: Record<SessionState, { color: string | null; beating: boolean; label: string }> = {
	idle: { color: null, beating: false, label: "" },
	starting: { color: "var(--warn)", beating: true, label: "Starting" },
	ready: { color: null, beating: false, label: "" },
	thinking: { color: "var(--accent)", beating: true, label: "Working" },
	error: { color: "var(--danger)", beating: false, label: "Error" },
	stopped: { color: null, beating: false, label: "" },
};

export function Rail({
	entries,
	selectedId,
	seen,
	connection,
	onSelect,
	onNew,
	onSettings,
	onEdit,
	onDelete,
	onHelp,
}: {
	entries: RosterEntry[];
	selectedId: string | null;
	/** The latest ts the window has already shown for each teammate. */
	seen: Record<string, number>;
	connection: Connection;
	onSelect(personaId: string): void;
	onNew(): void;
	onSettings(): void;
	onEdit(personaId: string): void;
	onDelete(personaId: string, name: string): void;
	onHelp(id: "shortcuts" | "about" | "github"): void;
}) {
	/* The help a menu bar would carry. Here because on Linux and Windows
	 * there is no menu bar, and a page nobody can reach is not a page. */
	const help: MenuEntry[] = [
		{ kind: "item", id: "shortcuts", text: "Keyboard shortcuts", onSelect: () => onHelp("shortcuts") },
		{ kind: "item", id: "about", text: "About Toad", onSelect: () => onHelp("about") },
		{ kind: "item", id: "github", text: "Toad on GitHub", onSelect: () => onHelp("github") },
	];
	return (
		<nav aria-label="Team" className="rail flex flex-col">
			<Band rail>
				<h1 className="eyebrow min-w-0 flex-1 truncate pl-1">Team</h1>
				<button
					type="button"
					className="control btn-icon"
					title={`New teammate (${chordKeys("new-teammate")})`}
					aria-label="New teammate"
					onClick={onNew}
				>
					<PlusIcon />
				</button>
			</Band>

			<div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2 pt-1">
				{entries.length === 0 ? (
					<p className="px-2 py-3 text-sm text-ink-3">
						No teammates yet. Each one keeps its own working directory, its own goal and its
						own conversation.
					</p>
				) : (
					entries.map((entry, index) => (
						<Row
							key={entry.persona.id}
							entry={entry}
							shortcut={index < 9 ? index + 1 : null}
							active={entry.persona.id === selectedId}
							unread={unreadOf(entry, selectedId, seen)}
							onSelect={() => onSelect(entry.persona.id)}
							onEdit={() => onEdit(entry.persona.id)}
							onDelete={() => onDelete(entry.persona.id, entry.persona.name)}
						/>
					))
				)}
			</div>

			<footer className="flex h-10 shrink-0 items-center gap-1 px-2">
				<button
					type="button"
					className="control btn-quiet -ml-1 gap-1.5 px-2 text-sm"
					title={`Settings (${chordKeys("settings")})`}
					onClick={onSettings}
				>
					<GearIcon className="text-ink-3" />
					Settings
				</button>
				<MenuButton className="control btn-icon" label="More" entries={help}>
					<MoreIcon />
				</MenuButton>
				{connection !== "open" && (
					<p role="status" className="instrument ml-auto flex min-w-0 items-center gap-1.5 truncate">
						<span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: "var(--warn)" }} />
						Reconnecting
					</p>
				)}
			</footer>
		</nav>
	);
}

function Row({
	entry,
	shortcut,
	active,
	unread,
	onSelect,
	onEdit,
	onDelete,
}: {
	entry: RosterEntry;
	shortcut: number | null;
	active: boolean;
	unread: boolean;
	onSelect(): void;
	onEdit(): void;
	onDelete(): void;
}) {
	const vital = VITAL[entry.session.state];
	const { preview, activity } = entry;
	const working = entry.session.state === "thinking" && activity !== undefined;
	const line = working
		? activity
		: preview
			? `${preview.from === "me" ? "You: " : ""}${oneLine(preview.text)}`
			: vital.label || oneLine(entry.persona.goal);
	return (
		<button
			type="button"
			data-teammate-row
			aria-current={active ? "true" : undefined}
			aria-label={`${entry.persona.name}${vital.label === "" ? "" : `, ${vital.label.toLowerCase()}`}${unread ? ", unread" : ""}`}
			className="rail-row group relative"
			onClick={onSelect}
			onContextMenu={(event) => {
				event.preventDefault();
				void popupTeammateMenu({ onOpen: onSelect, onEdit, onDelete });
			}}
		>
			{unread && (
				<span
					aria-hidden="true"
					className="absolute left-[3px] top-1/2 h-1.5 w-1.5 -translate-y-1/2 rounded-full bg-accent"
				/>
			)}
			<Avatar id={entry.persona.id} name={entry.persona.name} size={28} />
			<span className="min-w-0 flex-1">
				<span className="flex h-[18px] items-center gap-1.5">
					<span className={`min-w-0 flex-1 truncate ${unread ? "font-semibold text-ink" : "font-medium text-ink"}`}>
						{entry.persona.name}
					</span>
					{shortcut !== null && (
						<kbd
							aria-hidden="true"
							className="kbd opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
						>
							⌃{shortcut}
						</kbd>
					)}
					{vital.color !== null && (
						<span
							aria-hidden="true"
							className={`h-2 w-2 shrink-0 rounded-full ${vital.beating ? "beat" : ""}`}
							style={{ background: vital.color }}
						/>
					)}
				</span>
				<span
					className={`block h-4 truncate text-sm ${unread ? "text-ink-2" : "text-ink-3"} ${working ? "font-mono text-xs leading-4" : ""}`}
				>
					{line}
				</span>
			</span>
		</button>
	);
}

/** Off-screen, and the tape has a line newer than the last one this window showed. */
function unreadOf(
	entry: RosterEntry,
	selectedId: string | null,
	seen: Record<string, number>,
): boolean {
	if (entry.persona.id === selectedId) return false;
	const latest = entry.latest;
	if (latest == null) return false;
	const shown = seen[entry.persona.id];
	return shown == null || latest > shown;
}

/**
 * A preview is one line in a narrow rail, whatever shape it was written in.
 * The markdown an agent writes is rendered in the column and stripped here:
 * asterisks and backticks in a two-inch column are noise, not emphasis.
 */
function oneLine(source: string): string {
	return source
		.replace(/^\s{0,3}(```|~~~).*$/gm, "")
		.replace(/^\s{0,3}#{1,6}\s+/gm, "")
		.replace(/^\s{0,3}>\s?/gm, "")
		.replace(/^\s*([-*+]|\d{1,9}[.)])\s+/gm, "")
		.replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
		.replace(/[*_~`]/g, "")
		.replace(/\s+/g, " ")
		.trim();
}
