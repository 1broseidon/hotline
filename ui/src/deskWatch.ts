import { useSyncExternalStore } from "react";
import { unreadOf } from "./components/Rail";
import { activeDeskId, allDesks, deskKey, useDesks, wireFor } from "./desks";
import { noticeRoster } from "./notify";
import type { RosterEntry } from "./wire";

/**
 * The desks that are not on screen (BRO-145). The one on screen folds its
 * own roster (App.tsx); every other desk the window holds is watched here,
 * roster only, so a turn ending or a teammate stuck on a server still posts a
 * toast, and its unread rows still count in the dock's badge and beside its
 * name in the desk switcher.
 *
 * Unread is judged against the same per-desk "seen" the rail keeps, read
 * from storage, so opening that desk later agrees with what was counted.
 */

const SEEN_KEY = "hotline.rail.seen";

type Watched = { stop(): void; roster: RosterEntry[] };

const watched = new Map<string, Watched>();
let counts: Record<string, number> = {};
const listeners = new Set<() => void>();

function recount() {
	const next: Record<string, number> = {};
	for (const [deskId, one] of watched) {
		const seen = readSeen(deskId);
		// Nobody on a desk that is not on screen is being looked at, not even
		// the teammate it had open: a new line from them is unread too.
		next[deskId] = one.roster.filter((entry) => unreadOf(entry, null, seen)).length;
	}
	counts = next;
	for (const listener of listeners) listener();
}

function readSeen(deskId: string): Record<string, number> {
	try {
		const parsed: unknown = JSON.parse(localStorage.getItem(deskKey(SEEN_KEY, deskId)) ?? "{}");
		return typeof parsed === "object" && parsed !== null ? (parsed as Record<string, number>) : {};
	} catch {
		return {};
	}
}

function watch(deskId: string, name: string): Watched {
	const one: Watched = { stop: () => {}, roster: [] };
	const fold = (roster: RosterEntry[]) => {
		one.roster = roster;
		noticeRoster(roster, { id: deskId, name });
		recount();
	};
	const wire = wireFor(deskId);
	wire.connect();
	one.stop = wire.subscribe<RosterEntry>(
		{ view: "roster" },
		{
			snapshot: (entries) => fold(entries as RosterEntry[]),
			event: (entry) => {
				const at = one.roster.findIndex((known) => known.persona.id === entry.persona.id);
				fold(at === -1 ? [...one.roster, entry] : one.roster.map((known, index) => (index === at ? entry : known)));
			},
			removed: (personaId) => fold(one.roster.filter((known) => known.persona.id !== personaId)),
		},
	);
	return one;
}

/**
 * Watches every desk but the one on screen; called whenever either changes.
 * A desk coming on screen stops being watched here, since App folds it now.
 */
export function syncWatches(): void {
	const active = activeDeskId();
	const wanted = new Map(allDesks().filter((desk) => desk.id !== active).map((desk) => [desk.id, desk.name]));
	for (const [deskId, one] of watched) {
		if (!wanted.has(deskId)) {
			one.stop();
			watched.delete(deskId);
		}
	}
	for (const [deskId, name] of wanted) {
		if (!watched.has(deskId)) watched.set(deskId, watch(deskId, name));
	}
	recount();
}

/** Unread rows on each desk that is not on screen. */
export function useBackgroundUnread(): Record<string, number> {
	useDesks();
	return useSyncExternalStore(
		(listener) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		() => counts,
	);
}
