import { useSyncExternalStore } from "react";
import { unreadOf } from "./components/Rail";
import { activeDeskId, allDesks, deskKey, LOCAL_DESK, useDesks, wireFor } from "./desks";
import { noticeRoster } from "./notify";
import type { RosterEntry } from "./wire";

/**
 * The desks that are not on screen (BRO-145). The one on screen folds its
 * own roster (App.tsx); every other desk the window holds is watched here,
 * roster only, so a turn ending or a teammate stuck on a server still posts a
 * toast, its unread rows still count in the dock's badge and the desk
 * filter, and its teammates stand in the one rail with everyone else's
 * (BRO-178).
 *
 * Unread is judged against the same per-desk "seen" the rail keeps, read
 * from storage, so opening that desk later agrees with what was counted.
 */

const SEEN_KEY = "hotline.rail.seen";

/** A desk being watched, and the endpoint it was watched at. */
type Watched = { stop(): void; roster: RosterEntry[]; endpoint: string };

const watched = new Map<string, Watched>();
let counts: Record<string, number> = {};
let rosters: Record<string, RosterEntry[]> = {};
const listeners = new Set<() => void>();
const rosterListeners = new Set<() => void>();

function recount() {
	const next: Record<string, number> = {};
	for (const [deskId, one] of watched) {
		// Nobody on a desk that is not on screen is being looked at, not even
		// the teammate it had open: a new line from them is unread too.
		next[deskId] = one.roster.filter((entry) => unreadOf(entry, null, seenOn(deskId))).length;
	}
	// Every roster event on a watched desk comes here; only a count that
	// moved is news to the windows reading them.
	const same = Object.keys(next).length === Object.keys(counts).length && Object.entries(next).every(([deskId, count]) => counts[deskId] === count);
	if (same) return;
	counts = next;
	for (const listener of listeners) listener();
}

function republish() {
	const next: Record<string, RosterEntry[]> = {};
	for (const [deskId, one] of watched) next[deskId] = one.roster;
	rosters = next;
	for (const listener of rosterListeners) listener();
}

/** What this window has already shown of each teammate on a desk. */
export function seenOn(deskId: string): Record<string, number> {
	try {
		const parsed: unknown = JSON.parse(localStorage.getItem(deskKey(SEEN_KEY, deskId)) ?? "{}");
		return typeof parsed === "object" && parsed !== null ? (parsed as Record<string, number>) : {};
	} catch {
		return {};
	}
}

function watch(deskId: string, name: string, endpoint: string): Watched {
	const one: Watched = { stop: () => {}, roster: cachedRoster(deskId) ?? [], endpoint };
	const fold = (roster: RosterEntry[]) => {
		one.roster = roster;
		keepRoster(deskId, roster);
		noticeRoster(roster, { id: deskId, name });
		recount();
		republish();
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
 * A desk coming on screen stops being watched here, since App folds it now,
 * and a desk reached at a new endpoint is watched again there: its old
 * connection was closed with the old endpoint.
 */
export function syncWatches(): void {
	const active = activeDeskId();
	const wanted = new Map(allDesks().filter((desk) => desk.id !== active).map((desk) => [desk.id, desk]));
	for (const [deskId, one] of watched) {
		const desk = wanted.get(deskId);
		if (desk === undefined || one.endpoint !== endpointOf(desk)) {
			one.stop();
			watched.delete(deskId);
		}
	}
	for (const [deskId, desk] of wanted) {
		if (!watched.has(deskId)) watched.set(deskId, watch(deskId, desk.name, endpointOf(desk)));
	}
	recount();
	republish();
}

function endpointOf(desk: { origin: string; token: string }): string {
	return `${desk.origin} ${desk.token}`;
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

/** The teammates of each desk that is not on screen, as each last said. */
export function useBackgroundRosters(): Record<string, RosterEntry[]> {
	return useSyncExternalStore(
		(listener) => {
			rosterListeners.add(listener);
			return () => rosterListeners.delete(listener);
		},
		() => rosters,
	);
}

/** Each desk's last roster, for the moment after switching back to it. */
const rosterCache = new Map<string, RosterEntry[]>();
const ROSTER_KEY = "hotline.desk.roster";

/**
 * A remote desk's roster is also kept across restarts: a desk that can't be
 * reached at launch still shows its teammates as they last were, dimmed in
 * the rail, which is what the "Can't reach" band promises. The local desk is
 * always there.
 */
export function cachedRoster(deskId: string): RosterEntry[] | undefined {
	const held = rosterCache.get(deskId);
	if (held !== undefined || deskId === LOCAL_DESK) return held;
	try {
		const raw = localStorage.getItem(`${ROSTER_KEY}:${deskId}`);
		const kept = raw === null ? undefined : (JSON.parse(raw) as RosterEntry[]);
		if (Array.isArray(kept)) rosterCache.set(deskId, kept);
		return Array.isArray(kept) ? kept : undefined;
	} catch {
		return undefined;
	}
}

export function keepRoster(deskId: string, roster: RosterEntry[]) {
	rosterCache.set(deskId, roster);
	if (deskId === LOCAL_DESK) return;
	try {
		localStorage.setItem(`${ROSTER_KEY}:${deskId}`, JSON.stringify(roster));
	} catch {
		// Quota, private mode: the in-memory copy still serves this session.
	}
}
