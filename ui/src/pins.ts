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

/** One teammate in the window's team, and the desk it is on. */
export type TeamRow = { deskId: string; entry: RosterEntry };

/**
 * The team across every desk the rail shows (BRO-178). One desk reads as
 * `railOrder` always has. Several put each desk's pins first, in the desks'
 * order, and then everyone else together the way the phone lists them: who
 * is waiting on the person, then who spoke last, so a teammate on a server
 * is not below the fold just for living there.
 */
export function teamOrder(desks: { deskId: string; entries: RosterEntry[] }[]): { pinned: TeamRow[]; rest: TeamRow[]; all: TeamRow[] } {
	const ordered = desks.map(({ deskId, entries }) => {
		const { pinned, rest } = railOrder(entries);
		const row = (entry: RosterEntry): TeamRow => ({ deskId, entry });
		return { pinned: pinned.map(row), rest: rest.map(row) };
	});
	const pinned = ordered.flatMap((one) => one.pinned);
	const rest = ordered.flatMap((one) => one.rest);
	if (desks.length > 1) {
		rest.sort((a, b) => Number(b.entry.waiting) - Number(a.entry.waiting) || (b.entry.latest ?? 0) - (a.entry.latest ?? 0));
	}
	return { pinned, rest, all: [...pinned, ...rest] };
}

/**
 * The desks the rail lists and each one's teammates: every desk the window
 * holds, or the one the filter names, with the teammates of the desk on
 * screen as its own roster says and the others' as they were watched. A
 * window that knows no desks has the one on screen.
 */
export function shownTeam(
	desks: { id: string }[],
	here: string,
	filter: string | null,
	entries: RosterEntry[],
	elsewhere: Record<string, RosterEntry[]>,
): { deskId: string; entries: RosterEntry[] }[] {
	if (desks.length === 0) return [{ deskId: here, entries }];
	return desks
		.filter((desk) => filter === null || desk.id === filter)
		.map((desk) => ({ deskId: desk.id, entries: desk.id === here ? entries : (elsewhere[desk.id] ?? []) }));
}
