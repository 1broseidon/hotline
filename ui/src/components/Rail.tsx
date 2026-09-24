import { type FocusEvent, type MouseEvent, useEffect, useRef, useState } from "react";
import { chordKeys } from "../chords";
import { SettingsIcon, MoreIcon, PlusIcon } from "../icons";
import { popupTeammateMenu } from "../native";
import type { SessionState } from "../generated/contract";
import type { Connection, RosterEntry } from "../wire";
import { Avatar } from "../ui/Avatar";
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
	width,
	compact = false,
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
	/** The dragged width; none in a narrow window, where the rail is the whole window. */
	width?: number | undefined;
	/** Faces only: each name waits on a hover card. */
	compact?: boolean;
}) {
	const [tip, setTip] = useState<Tip | null>(null);
	const showTip = (row: HTMLElement | null, name = "", line = "") => {
		if (row === null) {
			setTip(null);
			return;
		}
		const box = row.getBoundingClientRect();
		setTip({ name, line, left: box.right + 8, top: box.top + box.height / 2 });
	};
	/* The help a menu bar would carry. Here because on Linux and Windows
	 * there is no menu bar, and a page nobody can reach is not a page. */
	const help: MenuEntry[] = [
		{ kind: "item", id: "shortcuts", text: "Keyboard shortcuts", onSelect: () => onHelp("shortcuts") },
		{ kind: "item", id: "about", text: "About Hotline", onSelect: () => onHelp("about") },
		{ kind: "item", id: "github", text: "Hotline on GitHub", onSelect: () => onHelp("github") },
	];
	return (
		<nav
			aria-label="Team"
			className={`rail flex flex-col ${compact ? "rail-compact" : ""}`}
			style={width !== undefined ? { width } : undefined}
		>
			<div className="min-h-0 flex-1 overflow-y-auto px-1 pb-2 pt-1" onScroll={() => setTip(null)}>
				{entries.length === 0 ? (
					compact ? null :
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
							{...(compact ? { onTip: showTip } : {})}
						/>
					))
				)}
			</div>

			<footer className={compact ? "flex shrink-0 flex-col items-center gap-1 pb-2" : "flex h-10 shrink-0 items-center gap-1 px-2.5"}>
				{compact ? (
					<button type="button" className="control btn-icon" title={`Settings (${chordKeys("settings")})`} aria-label="Settings" onClick={onSettings}>
						<SettingsIcon />
					</button>
				) : (
					<button
						type="button"
						className="control btn-quiet gap-1.5 px-2 text-sm"
						title={`Settings (${chordKeys("settings")})`}
						onClick={onSettings}
					>
						<SettingsIcon className="text-ink-3" />
						Settings
					</button>
				)}
				<button
					type="button"
					className="control btn-icon"
					title={`New teammate (${chordKeys("new-teammate")})`}
					aria-label="New teammate"
					onClick={onNew}
				>
					<PlusIcon />
				</button>
				<MenuButton className="control btn-icon" label="More" entries={help}>
					<MoreIcon />
				</MenuButton>
				{connection !== "open" && (
					<p
						role="status"
						className={`instrument flex min-w-0 items-center gap-1.5 truncate ${compact ? "h-6" : "ml-auto"}`}
						title={compact ? "Reconnecting" : undefined}
					>
						<span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: "var(--warn)" }} />
						{compact ? <span className="sr-only">Reconnecting</span> : "Reconnecting"}
					</p>
				)}
			</footer>
			{tip !== null && (
				<div role="tooltip" className="rail-tip" style={{ left: tip.left, top: tip.top }}>
					<span className="block truncate font-medium text-ink">{tip.name}</span>
					{tip.line !== "" && <span className="block truncate text-sm text-ink-3">{tip.line}</span>}
				</div>
			)}
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
	onTip,
}: {
	entry: RosterEntry;
	shortcut: number | null;
	active: boolean;
	unread: boolean;
	onSelect(): void;
	onEdit(): void;
	onDelete(): void;
	/** Faces only: the row shows its name and line on a card while hovered or focused. */
	onTip?: (row: HTMLElement | null, name?: string, line?: string) => void;
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
			{...(onTip !== undefined
				? {
						onMouseEnter: (event: MouseEvent<HTMLButtonElement>) => onTip(event.currentTarget, entry.persona.name, line),
						onFocus: (event: FocusEvent<HTMLButtonElement>) => onTip(event.currentTarget, entry.persona.name, line),
						onMouseLeave: () => onTip(null),
						onBlur: () => onTip(null),
					}
				: {})}
			onContextMenu={(event) => {
				event.preventDefault();
				void popupTeammateMenu({ onOpen: onSelect, onEdit, onDelete });
			}}
		>
			{unread && onTip === undefined && (
				<span
					aria-hidden="true"
					className="absolute left-[1px] top-1/2 h-1.5 w-1.5 -translate-y-1/2 rounded-full bg-accent"
				/>
			)}
			{onTip !== undefined ? (
				<span className="relative flex">
					<Avatar id={entry.persona.id} name={entry.persona.name} size={28} />
					{unread && <span aria-hidden="true" className="rail-face-unread" />}
					{vital.color !== null && (
						<span
							aria-hidden="true"
							className={`rail-face-vital ${vital.beating ? "beat" : ""}`}
							style={{ background: vital.color }}
						/>
					)}
				</span>
			) : (
			<Avatar id={entry.persona.id} name={entry.persona.name} size={28} />
			)}
			{onTip === undefined && (
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
			)}
		</button>
	);
}

/** The hover card of a row in the faces-only rail, placed beside the row in the window. */
type Tip = { name: string; line: string; left: number; top: number };

/** Off-screen, and the tape has a line newer than the last one this window showed. */
/** Whether a row shows as unread: the badge on the dock counts these (App.tsx). */
export function unreadOf(
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

/**
 * The rail's width, whether it is down to faces, and whether it is there
 * at all. Dragged at its edge through three stops — names, faces, gone —
 * closed and opened from the titlebar or the chord, and remembered by this
 * window: a list of names wants to be as wide as your longest name, on a
 * smaller screen the faces are enough, and sometimes you want the room.
 * Closing keeps the rest, so it comes back the way you left it.
 */
export type RailSize = { width: number; open: boolean; compact: boolean };

export const RAIL_WIDTH = 240;
export const RAIL_MIN = 180;
const RAIL_MAX = 420;
/** Faces only: a 28px face in the row's padding, in the rail's gutter. */
export const RAIL_FACES = 52;
/** Dragged narrower than this, the names give way to faces rather than squeezing. */
const RAIL_TO_FACES = 120;
/** And narrower than this, the faces go too. */
const RAIL_TO_GONE = 32;
const RAIL_STEP = 16;
const RAIL_KEY = "hotline.rail.size";

const clampWidth = (width: number) => Math.round(Math.min(RAIL_MAX, Math.max(RAIL_MIN, width)));

export function useRailSize(): [RailSize, (next: RailSize | ((was: RailSize) => RailSize)) => void] {
	const [size, setSize] = useState<RailSize>(loadRailSize);
	useEffect(() => {
		try {
			localStorage.setItem(RAIL_KEY, JSON.stringify(size));
		} catch {
			// Quota, private mode: the next launch opens at the default.
		}
	}, [size]);
	return [size, setSize];
}

function loadRailSize(): RailSize {
	try {
		const parsed: unknown = JSON.parse(localStorage.getItem(RAIL_KEY) ?? "null");
		if (typeof parsed === "object" && parsed !== null) {
			const { width, open } = parsed as Partial<RailSize>;
			return {
				width: typeof width === "number" && Number.isFinite(width) ? clampWidth(width) : RAIL_WIDTH,
				open: open !== false,
				compact: (parsed as Partial<RailSize>).compact === true,
			};
		}
	} catch {
		// Unreadable: the default.
	}
	return { width: RAIL_WIDTH, open: true, compact: false };
}

/** Where a drag that has got to `raw` pixels leaves the rail. */
function dragged(raw: number, size: RailSize): RailSize {
	if (raw < RAIL_TO_GONE) return { ...size, open: false };
	if (raw < RAIL_TO_FACES) return { ...size, open: true, compact: true };
	return { width: clampWidth(raw), open: true, compact: false };
}

/** What the rail is showing now, in pixels: 0 closed, the faces, or its width. */
const shown = (size: RailSize) => (!size.open ? 0 : size.compact ? RAIL_FACES : size.width);

/**
 * The rail's edge: the gutter between it and the pane, which you drag. It
 * stays in the window's left gutter while the rail is closed, so the rail
 * can be pulled back out the way it went. A double-click puts it back to
 * the default width with names; on the keyboard the arrows step through
 * widths, faces and gone, and Enter closes or opens it.
 */
export function RailEdge({ size, onSize }: { size: RailSize; onSize(next: RailSize): void }) {
	const [dragging, setDragging] = useState(false);
	const start = useRef({ x: 0, size });
	useEffect(() => {
		if (!dragging) return;
		document.documentElement.setAttribute("data-resizing", "");
		return () => document.documentElement.removeAttribute("data-resizing");
	}, [dragging]);
	return (
		<div
			role="separator"
			aria-orientation="vertical"
			aria-label="Resize the team"
			aria-valuemin={RAIL_MIN}
			aria-valuemax={RAIL_MAX}
			aria-valuenow={shown(size)}
			tabIndex={0}
			title="Drag to resize, or close"
			className="rail-edge"
			data-dragging={dragging || undefined}
			onPointerDown={(event) => {
				if (event.button !== 0) return;
				event.preventDefault();
				event.currentTarget.setPointerCapture(event.pointerId);
				start.current = { x: event.clientX, size };
				setDragging(true);
			}}
			onPointerMove={(event) => {
				if (!dragging) return;
				const began = start.current.size;
				// Measured from where the drag began, and anything it snaps to keeps
				// the width it began with, not one it passed on the way.
				onSize(dragged(shown(began) + event.clientX - start.current.x, began));
			}}
			onPointerUp={() => setDragging(false)}
			onPointerCancel={() => setDragging(false)}
			onDoubleClick={() => onSize({ width: RAIL_WIDTH, open: true, compact: false })}
			onKeyDown={(event) => {
				if (event.key === "ArrowLeft") {
					event.preventDefault();
					if (!size.open) return;
					if (size.compact) onSize({ ...size, open: false });
					else if (size.width - RAIL_STEP < RAIL_MIN) onSize({ ...size, compact: true });
					else onSize({ ...size, width: clampWidth(size.width - RAIL_STEP) });
				} else if (event.key === "ArrowRight") {
					event.preventDefault();
					if (!size.open) onSize({ ...size, open: true, compact: true });
					else if (size.compact) onSize({ ...size, compact: false });
					else onSize({ ...size, width: clampWidth(size.width + RAIL_STEP) });
				} else if (event.key === "Enter") {
					event.preventDefault();
					onSize({ ...size, open: !size.open });
				}
			}}
		/>
	);
}
