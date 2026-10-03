import type { SideThreadSummary } from "./generated/contract";

/**
 * The right-hand pane: side threads for the open teammate, and settings, one
 * at a time. Like the rail, its width is yours to drag, remembered by this
 * window, and put back to the default by a double-click on its edge.
 */
export const DOCK_WIDTH = 360;
export const DOCK_MIN = 300;
export const DOCK_MAX = 560;
export const DOCK_STEP = 16;
export const DOCK_KEY = "hotline.dock.size";
/** The narrowest the conversation is left when the pane stands beside it. */
export const CONVERSATION_MIN = 396;

export const clampDock = (width: number): number => Math.round(Math.min(DOCK_MAX, Math.max(DOCK_MIN, width)));

/** The remembered width, or the default when there is none or it is unreadable. */
export function loadDockWidth(storage: Pick<Storage, "getItem"> | undefined = safeStorage()): number {
	try {
		const parsed: unknown = JSON.parse(storage?.getItem(DOCK_KEY) ?? "null");
		if (typeof parsed === "object" && parsed !== null) {
			const { width } = parsed as { width?: unknown };
			if (typeof width === "number" && Number.isFinite(width)) return clampDock(width);
		}
	} catch {
		// Unreadable: the default.
	}
	return DOCK_WIDTH;
}

export function saveDockWidth(width: number, storage: Pick<Storage, "setItem"> | undefined = safeStorage()): void {
	try {
		storage?.setItem(DOCK_KEY, JSON.stringify({ width: clampDock(width) }));
	} catch {
		// Quota, private mode: the next launch opens at the default.
	}
}

function safeStorage(): Storage | undefined {
	try {
		return typeof localStorage === "undefined" ? undefined : localStorage;
	} catch {
		return undefined;
	}
}

/** Where a drag leaves the pane: its edge is on the left, so moving left widens it. */
export const draggedDock = (began: number, startX: number, x: number): number => clampDock(began + startX - x);

/**
 * Whether the pane has to lie over the conversation: beside it, the
 * conversation would be left narrower than it can be read at.
 */
export const dockOverlays = (mainWidth: number, dockWidth: number): boolean => mainWidth < dockWidth + 8 + CONVERSATION_MIN;

/** What a side thread is doing, as the list's dot says it. */
export type SideState = "running" | "waiting" | "parked" | "archived" | "idle";

export function sideState(side: Pick<SideThreadSummary, "status" | "working" | "waiting">): SideState {
	if (side.status === "archived") return "archived";
	if (side.waiting) return "waiting";
	if (side.status === "parked") return "parked";
	return side.working ? "running" : "idle";
}

/**
 * The list: threads still open (running, idle and parked) newest first, and
 * the archived ones folded apart, newest first by when they ended.
 */
export function groupSides(list: SideThreadSummary[]): { open: SideThreadSummary[]; archived: SideThreadSummary[] } {
	const stamp = (side: SideThreadSummary) => (side.status === "archived" ? (side.archivedAt ?? side.lastAt) : side.lastAt);
	const newest = (a: SideThreadSummary, b: SideThreadSummary) => stamp(b) - stamp(a);
	return {
		open: list.filter((side) => side.status !== "archived").sort(newest),
		archived: list.filter((side) => side.status === "archived").sort(newest),
	};
}

/** A time the way a list says it: now, 5m, 3h, 2d, then the date. */
export function relativeTime(at: number, now: number = Date.now()): string {
	const seconds = Math.max(0, Math.round((now - at) / 1000));
	if (seconds < 45) return "now";
	const minutes = Math.round(seconds / 60);
	if (minutes < 60) return `${minutes}m`;
	const hours = Math.round(minutes / 60);
	if (hours < 24) return `${hours}h`;
	const days = Math.round(hours / 24);
	if (days < 7) return `${days}d`;
	return new Date(at).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
