/**
 * Desktop toasts for the two moments that earn one: a turn ending, and a
 * teammate blocked.
 *
 * The judgement lives here, not in the shell, because the roster view already
 * carries the session edge and the last line, and a toast about the screen
 * already in your hand is noise. The shell posts (native.ts); this file only
 * decides. Phone push is a later seat — not this.
 */

import { getCurrentWindow } from "@tauri-apps/api/window";
import type { SessionState } from "./generated/contract";
import { postToast, requestAttention } from "./native";
import type { RosterEntry } from "./wire";

/** Last state seen per teammate, so a transition can be recognised as one. */
const lastState = new Map<string, SessionState>();

/**
 * A roster fold just arrived. Notify only on thinking → ready or thinking →
 * error, and only when this window is not focused. A snapshot, a teammate
 * that was never thinking, and a window you are looking at are all silent.
 * A teammate that blocked also bounces the dock once: a finished turn can
 * wait for the toast to be read, a stuck one is asking for a hand.
 */
export function noticeRoster(entries: RosterEntry[]): void {
	const live = new Set<string>();
	for (const entry of entries) {
		live.add(entry.persona.id);
		const previous = lastState.get(entry.persona.id);
		lastState.set(entry.persona.id, entry.session.state);
		if (previous !== "thinking") continue;
		if (entry.session.state !== "ready" && entry.session.state !== "error") continue;
		if (document.hasFocus()) continue;
		// A turn that ended on the person's own line said nothing to them:
		// a quiet schedule that found nothing stays quiet here too.
		if (entry.session.state === "ready" && entry.preview?.from === "me") continue;
		void postToast(entry.persona.id, entry.persona.name, lastLine(entry));
		if (entry.session.state === "error") void requestAttention();
	}
	for (const id of lastState.keys()) {
		if (!live.has(id)) lastState.delete(id);
	}
}

/** The chrome names who is open, so the task bar is the rail's selected row. */
export function windowTitle(name: string | null): string {
	return name === null ? "Hotline" : `${name} — Hotline`;
}

export function setWindowTitle(name: string | null): void {
	const title = windowTitle(name);
	document.title = title;
	try {
		void getCurrentWindow().setTitle(title);
	} catch {
		// A browser tab is not the desk; the document title is enough there.
	}
}

function lastLine(entry: RosterEntry): string {
	return (entry.preview?.text ?? "").replace(/\s+/g, " ").trim();
}
