import type { RosterEntry } from "./wire";

/** What a desk can pin; the core refuses a fourth. */
export const MAX_PINS = 3;

/**
 * The team of one desk as the rail shows it: the pinned teammates in their
 * slots, then everyone else in the desk's order (`arranged`). `all` is that
 * same order flat, which is what ⌃1–9 count along. `several` says the rail
 * lists more than one desk, where the teammates the desk has not placed
 * stand by who needs the person and who spoke last rather than in roster
 * order, so they interleave with the other desks' (`teamOrder`).
 */
export function railOrder(entries: RosterEntry[], several = false): { pinned: RosterEntry[]; rest: RosterEntry[]; all: RosterEntry[] } {
	const pinned = entries
		.filter((entry) => entry.pin != null)
		.sort((a, b) => (a.pin as number) - (b.pin as number))
		.slice(0, MAX_PINS);
	const others = entries.filter((entry) => !pinned.includes(entry));
	const rest = arranged(several ? others.slice().sort(byAttention) : others);
	return { pinned, rest, all: [...pinned, ...rest] };
}

/**
 * The list under the pins: the teammates the desk has placed, by place, and
 * after them the ones it has not, in the order they came. A desk nobody has
 * arranged has no places, so it reads exactly as it did before there were
 * any.
 *
 * A placed teammate that is waiting on the person keeps its place, and its
 * waiting mark is what says so: once the person has put the list in order,
 * the list does not move under them. This is the one place that rule lives.
 */
function arranged(rest: RosterEntry[]): RosterEntry[] {
	const placed = rest.filter((entry) => entry.place != null).sort((a, b) => (a.place as number) - (b.place as number));
	return [...placed, ...rest.filter((entry) => entry.place == null)];
}

/** Who is waiting on the person first, then who spoke last. */
function byAttention(a: { waiting: boolean; latest?: number }, b: { waiting: boolean; latest?: number }): number {
	return Number(b.waiting) - Number(a.waiting) || (b.latest ?? 0) - (a.latest ?? 0);
}

/**
 * Where a teammate lands when it is moved one place, or null at the end of
 * its row of pins or its desk's list. The menu's Move left / right and Move
 * up / down, for the keyboard and where the platform does not hand a page
 * its own drag events.
 */
export function nudged(at: number, direction: -1 | 1, count: number): number | null {
	const next = at + direction;
	return next < 0 || next >= count ? null : next;
}

/**
 * A desk's order once `id` is moved to `index` in it: lifted out, then put
 * in at that index, as a pin is given a slot. `ids` is the desk's list under
 * the pins as the rail shows it, so the first move fixes the order that is
 * on screen and moves only the one teammate.
 */
export function movedTo(ids: string[], id: string, index: number): string[] {
	const others = ids.filter((one) => one !== id);
	others.splice(Math.max(0, Math.min(index, others.length)), 0, id);
	return others;
}

/** One teammate in the window's team, and the desk it is on. */
export type TeamRow = { deskId: string; entry: RosterEntry };

/**
 * The team across every desk the rail shows (BRO-178). One desk reads as
 * `railOrder` always has. Several put each desk's pins first, in the desks'
 * order, and then everyone else together the way the phone lists them: who
 * is waiting on the person, then who spoke last, so a teammate on a server
 * is not below the fold just for living there. A desk's own order holds
 * among its own rows: the lists are merged by the teammate at the head of
 * each, so a desk nobody arranged sorts exactly as before and an arranged
 * one keeps its teammates in the order it put them.
 */
export function teamOrder(desks: { deskId: string; entries: RosterEntry[] }[]): { pinned: TeamRow[]; rest: TeamRow[]; all: TeamRow[] } {
	const ordered = desks.map(({ deskId, entries }) => {
		const { pinned, rest } = railOrder(entries, desks.length > 1);
		const row = (entry: RosterEntry): TeamRow => ({ deskId, entry });
		return { pinned: pinned.map(row), rest: rest.map(row) };
	});
	const pinned = ordered.flatMap((one) => one.pinned);
	const rest = merged(ordered.map((one) => one.rest));
	return { pinned, rest, all: [...pinned, ...rest] };
}

/**
 * Several desks' lists as one, each keeping its own order: the next row is
 * the head of whichever list `byAttention` puts first, the earlier desk on a
 * tie. Lists that are each already in that order come out as one sort of
 * them all would.
 */
function merged(lists: TeamRow[][]): TeamRow[] {
	const queues = lists.map((list) => list.slice());
	const out: TeamRow[] = [];
	for (;;) {
		let first: TeamRow[] | undefined;
		for (const queue of queues) {
			const head = queue[0];
			const best = first?.[0];
			if (head !== undefined && (best === undefined || byAttention(head.entry, best.entry) < 0)) first = queue;
		}
		const next = first?.shift();
		if (next === undefined) return out;
		out.push(next);
	}
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
