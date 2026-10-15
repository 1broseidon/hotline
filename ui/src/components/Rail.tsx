import { type ReactNode, type FocusEvent, type MouseEvent, useEffect, useRef, useState } from "react";
import { chordKeys } from "../chords";
import { MAX_PINS, nudged, shownTeam, teamOrder, type TeamRow } from "../pins";
import { ChevronDownIcon, ComputerIcon } from "../icons";
import { popupTeammateMenu } from "../native";
import { shownState } from "../activity";
import type { SessionState } from "../generated/contract";
import type { Connection, RosterEntry } from "../wire";
import { Avatar, faceOf, initialOf } from "../ui/Avatar";
import { MenuButton, type MenuEntry } from "../ui/Menu";
import { seenOn, useBackgroundUnread } from "../deskWatch";
import { linkWords, useDeskLinks } from "../deskLinks";
import { setDeskFilter, useActiveDesk, useDeskFilter, useDesks, type Desk } from "../desks";
import { useSaved } from "../useSaved";

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
	elsewhere = {},
	loaded = true,
	selectedId,
	seen,
	connection,
	onSelect,
	onEdit,
	onDelete,
	onPin,
	onAddDesk,
	width,
	compact = false,
	search,
	deskKey,
	deskCard,
}: {
	/** The teammates of the desk on screen. */
	entries: RosterEntry[];
	/** The teammates of every other desk this window holds, by desk: one rail lists them all (BRO-178). */
	elsewhere?: Record<string, RosterEntry[]>;
	/** Whether the desk's roster has arrived; before it, no teammates is not "no teammates yet". */
	loaded?: boolean;
	selectedId: string | null;
	/** The latest ts the window has already shown for each teammate on the desk on screen. */
	seen: Record<string, number>;
	connection: Connection;
	/** A teammate picked; `deskId` names its desk when that is not the desk on screen. */
	onSelect(personaId: string, deskId?: string): void;
	onEdit(personaId: string, deskId?: string): void;
	onDelete(personaId: string, name: string, deskId?: string): void;
	/** Pin at a slot, or unpin with null. */
	onPin(personaId: string, slot: number | null, deskId?: string): void;
	onAddDesk(): void;
	/** The dragged width; none in a narrow window, where the rail is the whole window. */
	width?: number | undefined;
	/** Faces only: each name waits on a hover card. */
	compact?: boolean;
	/** The dock at the rail's foot: the team's search, and the desk's call key. */
	search: ReactNode;
	deskKey: ReactNode;
	/** A call to the desk, as a card that grows up out of the dock. */
	deskCard?: ReactNode;
}) {
	const [tip, setTip] = useState<Tip | null>(null);
	const words = connection === "outdated" ? "This window needs a newer Hotline core" : "Reconnecting";
	/* The pinned face being dragged, and the slot it would drop into. */
	const [drag, setDrag] = useState<{ id: string; over: number | null } | null>(null);
	const desks = useDesks();
	const active = useActiveDesk();
	const filter = useDeskFilter();
	const here = active?.id ?? "";
	const { pinned, rest } = teamOrder(shownTeam(desks, here, filter, entries, elsewhere));
	/* Each row's desk is said only when there is more than one to tell apart. */
	const deskOf = (row: TeamRow) => desks.find((desk) => desk.id === row.deskId);
	const shownDesk = (row: TeamRow) => (desks.length > 1 ? shortName(deskOf(row)?.name ?? "") : undefined);
	const offline = (row: TeamRow) => {
		const desk = deskOf(row);
		return desk?.kind === "remote" && desk.state !== undefined && desk.state !== "open";
	};
	const onDesk = (row: TeamRow) => row.deskId === here;
	const unread = (row: TeamRow) =>
		onDesk(row) ? unreadOf(row.entry, selectedId, seen) : unreadOf(row.entry, null, seenOn(row.deskId));
	const elsewhereId = (row: TeamRow) => (onDesk(row) ? undefined : row.deskId);
	const key = (row: TeamRow) => `${row.deskId}/${row.entry.persona.id}`;
	const showTip = (row: HTMLElement | null, name = "", line = "") => {
		if (row === null) {
			setTip(null);
			return;
		}
		const box = row.getBoundingClientRect();
		setTip({ name, line, left: box.right + 8, top: box.top + box.height / 2 });
	};
	return (
		<nav
			aria-label="Team"
			className={`rail flex flex-col ${compact ? "rail-compact" : ""}`}
			style={width !== undefined ? { width } : undefined}
		>
			<DeskFilter compact={compact} onAdd={onAddDesk} />
			<div className="min-h-0 flex-1 overflow-y-auto px-1 pb-2 pt-1" onScroll={() => setTip(null)}>
				{pinned.length + rest.length === 0 ? (
					compact || !loaded ? null :
					<p className="px-2 py-3 text-sm text-ink-3">
						No teammates yet. Each one keeps its own working directory, its own goal and its
						own conversation.
					</p>
				) : (
					<>
						{pinned.length > 0 && (
							<div className={compact ? "rail-pins rail-pins-stacked" : "rail-pins"} role="group" aria-label="Pinned">
								{pinned.map((row, index) => {
									const { entry } = row;
									// A pin's slot is on its own desk's row of pins.
									const ownPins = pinned.filter((one) => one.deskId === row.deskId);
									const slot = ownPins.indexOf(row);
									return (
										<PinTile
											key={key(row)}
											entry={entry}
											desk={shownDesk(row)}
											offline={offline(row)}
											slot={slot}
											count={ownPins.length}
											shortcut={index < 9 ? index + 1 : null}
											active={onDesk(row) && entry.persona.id === selectedId}
											unread={unread(row)}
											dragging={drag?.id === key(row)}
											over={drag !== null && drag.id !== key(row) && drag.id.startsWith(`${row.deskId}/`) && drag.over === slot}
											compact={compact}
											onSelect={() => onSelect(entry.persona.id, elsewhereId(row))}
											onEdit={() => onEdit(entry.persona.id, elsewhereId(row))}
											onDelete={() => onDelete(entry.persona.id, entry.persona.name, elsewhereId(row))}
											onPin={(personaId, at) => {
												// A pin moves along its own desk's row and no other.
												if (drag !== null && !drag.id.startsWith(`${row.deskId}/`)) return;
												onPin(personaId, at, elsewhereId(row));
											}}
											onDrag={(dragged) =>
												// Which face is being dragged is known when the drag starts;
												// passing over another face only moves where it would land.
												setDrag((was) =>
													dragged === null
														? null
														: { id: was !== null && dragged.over !== null ? was.id : `${row.deskId}/${dragged.id}`, over: dragged.over },
												)
											}
											onTip={showTip}
										/>
									);
								})}
							</div>
						)}
						{rest.map((row, index) => {
							const { entry } = row;
							const pins = pinned.filter((one) => one.deskId === row.deskId).length;
							return (
								<Row
									key={key(row)}
									entry={entry}
									desk={shownDesk(row)}
									offline={offline(row)}
									shortcut={pinned.length + index < 9 ? pinned.length + index + 1 : null}
									active={onDesk(row) && entry.persona.id === selectedId}
									unread={unread(row)}
									full={pins >= MAX_PINS}
									onSelect={() => onSelect(entry.persona.id, elsewhereId(row))}
									onEdit={() => onEdit(entry.persona.id, elsewhereId(row))}
									onDelete={() => onDelete(entry.persona.id, entry.persona.name, elsewhereId(row))}
									onPin={() => onPin(entry.persona.id, pins, elsewhereId(row))}
									{...(compact ? { onTip: showTip } : {})}
								/>
							);
						})}
					</>
				)}
			</div>

			{deskCard}
			{/* The dock: search, and the desk's line. Settings and a new teammate are the title bar's. */}
			<footer className={compact ? "rail-dock rail-dock-compact" : "rail-dock"}>
				{search}
				{deskKey}
			</footer>
			{connection !== "open" && (
				<p
					role="status"
					className={`instrument flex min-w-0 shrink-0 items-center gap-1.5 truncate ${compact ? "h-6 justify-center" : "px-3.5 pb-2"}`}
					title={compact || connection === "outdated" ? words : undefined}
				>
					<span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: "var(--warn)" }} />
					{compact ? <span className="sr-only">{words}</span> : words}
				</p>
			)}
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
	desk,
	offline = false,
	shortcut,
	active,
	unread,
	full,
	onSelect,
	onEdit,
	onDelete,
	onPin,
	onTip,
}: {
	entry: RosterEntry;
	/** Its desk's name, when the rail lists more than one desk. */
	desk?: string | undefined;
	/** Its desk cannot be reached: the row stays, dimmed, as it last was. */
	offline?: boolean;
	shortcut: number | null;
	active: boolean;
	unread: boolean;
	/** The desk already has its three pins. */
	full: boolean;
	onSelect(): void;
	onEdit(): void;
	onDelete(): void;
	onPin(): void;
	/** Faces only: the row shows its name and line on a card while hovered or focused. */
	onTip?: (row: HTMLElement | null, name?: string, line?: string) => void;
}) {
	const state = shownState(entry.session);
	const vital = VITAL[state];
	const working = state === "thinking" && entry.activity !== undefined;
	const busy = entry.session.state === "starting" || entry.drawing;
	const line = lineOf(entry);
	const named = desk === undefined ? entry.persona.name : `${entry.persona.name} · ${desk}`;
	return (
		<button
			type="button"
			data-teammate-row
			data-offline={offline || undefined}
			data-working={working || undefined}
			aria-current={active ? "true" : undefined}
			aria-label={`${named}${vital.label === "" ? "" : `, ${vital.label.toLowerCase()}`}${unread ? ", unread" : ""}${offline ? ", unreachable" : ""}`}
			className="rail-row group relative"
			onClick={onSelect}
			{...(onTip !== undefined
				? {
						onMouseEnter: (event: MouseEvent<HTMLButtonElement>) => onTip(event.currentTarget, named, line),
						onFocus: (event: FocusEvent<HTMLButtonElement>) => onTip(event.currentTarget, named, line),
						onMouseLeave: () => onTip(null),
						onBlur: () => onTip(null),
					}
				: {})}
			onContextMenu={(event) => {
				event.preventDefault();
				void popupTeammateMenu({ onOpen: onSelect, onEdit, onDelete, pin: { pinned: false, full, onToggle: onPin } });
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
					<Avatar id={entry.persona.id} name={entry.persona.name} size={28} hash={entry.persona.avatar?.hash} busy={busy} />
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
			<Avatar id={entry.persona.id} name={entry.persona.name} size={32} hash={entry.persona.avatar?.hash} busy={busy} />
			)}
			{onTip === undefined && (
			<span className="min-w-0 flex-1">
				<span className="flex h-[18px] items-center gap-1.5">
					<span className={`min-w-0 flex-1 truncate ${unread ? "font-semibold text-ink" : "font-medium text-ink"}`}>
						{entry.persona.name}
						{desk !== undefined && <span className="rail-desk">{desk}</span>}
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

/** What a row says under the name: the tool running, the last line, or the goal. */
function lineOf(entry: RosterEntry): string {
	const state = shownState(entry.session);
	const vital = VITAL[state];
	const { preview, activity } = entry;
	if (state === "thinking" && activity !== undefined) return activity;
	return preview
		? `${preview.from === "me" ? "You: " : ""}${oneLine(preview.text)}`
		: vital.label || oneLine(entry.persona.goal);
}

/**
 * A pinned teammate: a larger face with the name under it, carrying what its
 * row did — the vital, the unread dot, the busy ring. In the faces-only rail
 * the tiles stack and the name waits on the hover card. Dragged along the
 * row to reorder; the menu's Move left / right does the same for the keyboard
 * and for a window that does not hand the page its drag events.
 */
function PinTile({
	entry,
	desk,
	offline = false,
	slot,
	count,
	shortcut,
	active,
	unread,
	dragging,
	over,
	compact,
	onSelect,
	onEdit,
	onDelete,
	onPin,
	onDrag,
	onTip,
}: {
	entry: RosterEntry;
	/** Its desk's name, when the rail lists more than one desk. */
	desk?: string | undefined;
	offline?: boolean;
	slot: number;
	count: number;
	shortcut: number | null;
	active: boolean;
	unread: boolean;
	dragging: boolean;
	over: boolean;
	compact: boolean;
	onSelect(): void;
	onEdit(): void;
	onDelete(): void;
	onPin(personaId: string, slot: number | null): void;
	onDrag(drag: { id: string; over: number | null } | null): void;
	onTip(row: HTMLElement | null, name?: string, line?: string): void;
}) {
	const { persona } = entry;
	const vital = VITAL[entry.session.state];
	const busy = entry.session.state === "starting" || entry.drawing;
	const line = lineOf(entry);
	const named = desk === undefined ? persona.name : `${persona.name} · ${desk}`;
	const left = nudged(slot, -1, count);
	const right = nudged(slot, 1, count);
	return (
		<button
			type="button"
			data-teammate-row
			data-pin={slot}
			data-offline={offline || undefined}
			draggable
			aria-current={active ? "true" : undefined}
			aria-label={`${named}, pinned${vital.label === "" ? "" : `, ${vital.label.toLowerCase()}`}${unread ? ", unread" : ""}${offline ? ", unreachable" : ""}`}
			title={compact || shortcut === null ? undefined : `${named} (${chordKeys("teammate-seat").replace("1–9", String(shortcut))})`}
			className="rail-pin group"
			data-dragging={dragging || undefined}
			data-over={over || undefined}
			onClick={onSelect}
			onMouseEnter={compact ? (event) => onTip(event.currentTarget, named, line) : undefined}
			onFocus={compact ? (event) => onTip(event.currentTarget, named, line) : undefined}
			onMouseLeave={compact ? () => onTip(null) : undefined}
			onBlur={compact ? () => onTip(null) : undefined}
			onDragStart={(event) => {
				event.dataTransfer.setData("text/plain", persona.id);
				event.dataTransfer.effectAllowed = "move";
				onTip(null);
				onDrag({ id: persona.id, over: null });
			}}
			onDragOver={(event) => {
				event.preventDefault();
				event.dataTransfer.dropEffect = "move";
				onDrag({ id: event.dataTransfer.getData("text/plain") || persona.id, over: slot });
			}}
			onDrop={(event) => {
				event.preventDefault();
				const id = event.dataTransfer.getData("text/plain");
				onDrag(null);
				if (id !== "" && id !== persona.id) onPin(id, slot);
			}}
			onDragEnd={() => onDrag(null)}
			onContextMenu={(event) => {
				event.preventDefault();
				void popupTeammateMenu({
					onOpen: onSelect,
					onEdit,
					onDelete,
					pin: {
						pinned: true,
						full: true,
						onToggle: () => onPin(persona.id, null),
						...(left !== null ? { onLeft: () => onPin(persona.id, left) } : {}),
						...(right !== null ? { onRight: () => onPin(persona.id, right) } : {}),
					},
				});
			}}
		>
			<span className="relative flex">
				<Avatar id={persona.id} name={persona.name} size={compact ? 36 : 52} hash={persona.avatar?.hash} busy={busy} />
				{unread && <span aria-hidden="true" className="rail-face-unread" />}
				{vital.color !== null && (
					<span
						aria-hidden="true"
						className={`rail-face-vital ${vital.beating ? "beat" : ""}`}
						style={{ background: vital.color }}
					/>
				)}
			</span>
			{!compact && (
				<span className={`rail-pin-name text-sm ${unread ? "font-semibold text-ink" : "font-medium text-ink-2"}`}>{persona.name}</span>
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
	useSaved(size, (kept) => {
		try {
			localStorage.setItem(RAIL_KEY, JSON.stringify(kept));
		} catch {
			// Quota, private mode: the next launch opens at the default.
		}
	});
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

/**
 * Which desks' teammates the rail lists, when the window holds more than
 * one desk: this computer's and the remote desks it is paired with
 * (BRO-145). All of them by default, or one. Narrowing the list never swaps
 * the window: what is open stays open. Each desk says how it is doing — and
 * whether this computer's teammates are linked with it (ADR 0003) — the
 * way the phone's desk menu does.
 */
function DeskFilter({ compact = false, onAdd }: { compact?: boolean | undefined; onAdd(): void }) {
	const desks = useDesks();
	if (desks.length < 2) return null;
	return <DeskFilterMenu desks={desks} compact={compact} onAdd={onAdd} />;
}

function DeskFilterMenu({ desks, compact, onAdd }: { desks: Desk[]; compact: boolean; onAdd(): void }) {
	const filter = useDeskFilter();
	const unread = useBackgroundUnread();
	const links = useDeskLinks();
	const shown = desks.find((desk) => desk.id === filter) ?? null;
	const away = desks.some((desk) => desk.kind === "remote" && desk.state !== undefined && desk.state !== "open");
	const elsewhere = desks.some((desk) => desk.id !== shown?.id && (unread[desk.id] ?? 0) > 0);
	const entries: MenuEntry[] = [{ kind: "item", id: "all", text: "All desks", checked: filter === null, onSelect: () => setDeskFilter(null) }];
	let group: string | undefined;
	for (const desk of desks) {
		const here = desk.kind === "local" ? "On this computer" : "Remote";
		if (here !== group) {
			entries.push({ kind: "rule" }, { kind: "heading", text: here });
			group = here;
		}
		const link = links.find((one) => one.deskId === desk.id);
		entries.push({
			kind: "item",
			id: desk.id,
			text: desk.name,
			detail: [unread[desk.id] ? `${unread[desk.id]} unread` : undefined, deskDetail(desk), link === undefined ? undefined : linkWords(link)].filter(Boolean).join(" · "),
			checked: desk.id === filter,
			onSelect: () => setDeskFilter(desk.id),
		});
	}
	entries.push({ kind: "rule" }, { kind: "item", id: "add-desk", text: "Add a server…", onSelect: onAdd });
	const name = shown === null ? "All desks" : shown.name;
	return (
		<div className={`shrink-0 pt-2 ${compact ? "flex justify-center" : "px-2"}`}>
			<MenuButton
				className={compact ? "account account-compact" : "account"}
				align="start"
				label={`Desks: ${name}`}
				title={shown === null ? `All desks${away ? " · one offline" : ""}` : `${shown.name} · ${deskDetail(shown)}`}
				entries={entries}
			>
				{shown === null ? <AllTile away={away} count={desks.length} /> : <DeskTile desk={shown} />}
				{!compact && <span className="account-name">{shown === null ? name : shortName(name)}</span>}
				{elsewhere && <span className="account-unread" aria-label="Unread on a desk not shown" />}
				{!compact && <ChevronDownIcon className="account-chevron" />}
			</MenuButton>
		</div>
	);
}

/** Every desk at once: how many, and a mark when one of them can't be reached. */
function AllTile({ away, count }: { away: boolean; count: number }) {
	return (
		<span aria-hidden="true" className="account-tile" data-local>
			{count}
			{away && <span className="account-state" data-state="unreachable" />}
		</span>
	);
}

/** A desk's face: this computer's is the screen, a remote one its initial, on a rounded square so it never reads as a teammate. */
function DeskTile({ desk }: { desk: Desk }) {
	return (
		<span aria-hidden="true" className="account-tile" style={{ background: desk.kind === "local" ? undefined : faceOf(desk.id) }} data-local={desk.kind === "local" || undefined}>
			{desk.kind === "local" ? <ComputerIcon /> : initialOf(desk.name)}
			{desk.state !== undefined && desk.state !== "open" && <span className="account-state" data-state={desk.state} />}
		</span>
	);
}

/** A host name reads by its first label: s223265.example.net is s223265. */
function shortName(name: string): string {
	return /^[\w-]+(\.[\w-]+)+$/.test(name) ? (name.split(".")[0] ?? name) : name;
}

function deskDetail(desk: Desk): string {
	if (desk.kind === "local") return "This computer";
	switch (desk.state) {
		case "unreachable":
			return "Can't reach it";
		case "revoked":
			return "No longer paired";
		case "connecting":
			return "Connecting…";
		default:
			return "Remote";
	}
}
