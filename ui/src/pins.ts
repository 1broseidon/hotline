import type { RosterEntry } from "./wire";

/** What a desk can pin; the core refuses a fourth. */
export const MAX_PINS = 3;

/**
 * The team as the rail shows it: the pinned teammates in their slots, then
 * everyone else in roster order. `all` is that same order flat, which is what
 * ⌃1–9 count along.
 */
export function railOrder(entries: RosterEntry[]): { pinned: RosterEntry[]; rest: RosterEntry[]; all: RosterEntry[] } {
	const pinned = entries
		.filter((entry) => entry.pin != null)
		.sort((a, b) => (a.pin as number) - (b.pin as number))
		.slice(0, MAX_PINS);
	const rest = entries.filter((entry) => !pinned.includes(entry));
	return { pinned, rest, all: [...pinned, ...rest] };
}

/**
 * Where a pinned teammate lands when it is moved one place, or null at the
 * end of the row. The menu's "Move left / right", for the keyboard and where
 * the platform does not hand a page its own drag events.
 */
export function nudged(slot: number, direction: -1 | 1, pinned: number): number | null {
	const next = slot + direction;
	return next < 0 || next >= pinned ? null : next;
}
