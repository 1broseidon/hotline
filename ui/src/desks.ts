import { useSyncExternalStore } from "react";
import { Wire } from "./wire";

/**
 * The desks this window can show: the one on this computer, if there is
 * one, and any remote desk it is paired with (BRO-145). The shell says
 * which, as `window.__hotlineDesks`; a shell from before remote desks sets
 * only `__hotlineDesk`, and that one is the local desk.
 *
 * Every desk is reached the same way, a WebSocket speaking the wire
 * contract at `origin` with `token`: the local Door, or the loopback bridge
 * the shell keeps open to a remote desk. So a desk is only an endpoint and
 * a name, and the window never needs to know which kind it is talking to
 * except to say so.
 *
 * One desk is active: the one whose teammates, settings and computers are
 * on screen. The rest of the window's code talks to "the desk" and reaches
 * the active one (see `wire` in wire.ts); switching desks remounts what is
 * on screen, so nothing keeps talking to the one left behind.
 */
export type DeskKind = "local" | "remote";

/** How a remote desk's bridge is doing. A local desk is always reachable while the window runs. */
export type DeskState = "connecting" | "open" | "unreachable" | "revoked";

export type Desk = {
	id: string;
	name: string;
	kind: DeskKind;
	origin: string;
	token: string;
	state?: DeskState;
};

declare global {
	interface Window {
		__hotlineDesks?: Desk[];
	}
}

/** The id the local desk goes by, so a choice of it survives restarts. */
export const LOCAL_DESK = "local";
const ACTIVE_KEY = "hotline.desk.active";

/* Read lazily and guarded: tests import this module before a DOM exists. */
const saved = {
	get: () => (typeof localStorage === "undefined" ? null : localStorage.getItem(ACTIVE_KEY)),
	set: (id: string) => {
		if (typeof localStorage !== "undefined") localStorage.setItem(ACTIVE_KEY, id);
	},
};

function initial(): Desk[] {
	if (typeof window === "undefined") return [];
	const listed = window.__hotlineDesks;
	if (listed !== undefined && listed.length > 0) return listed;
	const local = window.__hotlineDesk;
	return local ? [{ id: LOCAL_DESK, name: "This computer", kind: "local", origin: local.origin, token: local.token }] : [];
}

let desks: Desk[] = initial();
let active: string | null = pick(saved.get());
const listeners = new Set<() => void>();
const wires = new Map<string, Wire>();

/** The remembered desk if it is still here, else the local one, else the first. */
function pick(wanted: string | null): string | null {
	if (wanted !== null && desks.some((desk) => desk.id === wanted)) return wanted;
	return desks.find((desk) => desk.kind === "local")?.id ?? desks[0]?.id ?? null;
}

function changed() {
	for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

/** The connection to one desk, made on first use and kept for the window's life. */
export function wireFor(deskId: string): Wire {
	let one = wires.get(deskId);
	if (one === undefined) {
		const desk = desks.find((candidate) => candidate.id === deskId);
		if (desk === undefined) throw new Error(`No desk ${deskId} in this window.`);
		one = new Wire({ origin: desk.origin, token: desk.token });
		wires.set(deskId, one);
	}
	return one;
}

export function allDesks(): Desk[] {
	return desks;
}

export function activeDeskId(): string | null {
	return active;
}

export function setActiveDesk(deskId: string) {
	if (deskId === active || !desks.some((desk) => desk.id === deskId)) return;
	active = deskId;
	saved.set(deskId);
	changed();
}

/**
 * The shell's new list: a desk paired, removed, or a bridge's state moved.
 * A desk whose endpoint changed gets a fresh connection; one that left is
 * closed. If the active desk left, another one takes its place.
 */
export function replaceDesks(next: Desk[]) {
	for (const [id, one] of wires) {
		const now = next.find((desk) => desk.id === id);
		const before = desks.find((desk) => desk.id === id);
		if (now === undefined || before === undefined || now.origin !== before.origin || now.token !== before.token) {
			one.close();
			wires.delete(id);
		}
	}
	desks = next;
	active = pick(active);
	changed();
}

export function useDesks(): Desk[] {
	return useSyncExternalStore(subscribe, () => desks);
}

export function useActiveDesk(): Desk | null {
	return useSyncExternalStore(subscribe, () => desks.find((desk) => desk.id === active) ?? null);
}
