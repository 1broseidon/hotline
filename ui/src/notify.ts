/**
 * Desktop toasts for the two moments that earn one: a turn ending, and a
 * teammate blocked, plus a post from another thread.
 *
 * The judgement lives here, not in the shell, because the roster view already
 * carries the session edge and the last line, and a toast about the screen
 * already in your hand is noise. The shell posts (native.ts); this file only
 * decides. Phone push is a later seat — not this.
 */

import { invoke } from "@tauri-apps/api/core";
import { shownState } from "./activity";
import type { SessionState } from "./generated/contract";
import { postToast, requestAttention } from "./native";
import type { RosterEntry } from "./wire";

/** Last state seen per desk and teammate, so a transition can be recognised as one. */
const lastState = new Map<string, SessionState>();
const lastPost = new Map<string, number | undefined>();

/**
 * A roster fold just arrived. Notify only on thinking → ready or thinking →
 * error, or a new thread post, and only when you are not looking at it: the window is unfocused,
 * or the desk is not the one on screen. A snapshot, a teammate that was never
 * thinking, and a desk you are looking at are all silent.
 * A teammate that blocked also bounces the dock once: a finished turn can
 * wait for the toast to be read, a stuck one is asking for a hand.
 *
 * `desk` scopes it to one desk's roster. A desk that is not the one on
 * screen (deskWatch.ts) names itself in the toast, and its click says
 * which desk to open (`toastTarget`).
 */
export function noticeRoster(entries: RosterEntry[], desk: { id: string; name?: string } = { id: "" }): void {
	const live = new Set<string>();
	const scope = `${desk.id}\u0000`;
	for (const entry of entries) {
		const key = scope + entry.persona.id;
		live.add(key);
		// A turn open only for its subagents has replied: it reads as ended.
		const state = shownState(entry.session);
		const previous = lastState.get(key);
		lastState.set(key, state);
		const posted = lastPost.has(key) && entry.preview?.posted === true && lastPost.get(key) !== entry.latest;
		lastPost.set(key, entry.latest);
		if (!posted && previous !== "thinking") continue;
		if (!posted && state !== "ready" && state !== "error") continue;
		// Focus means the desk on screen is being looked at; a desk off screen
		// is not, however focused the window is.
		if (desk.name === undefined && document.hasFocus()) continue;
		// A turn that ended on the person's own line said nothing to them:
		// a quiet schedule that found nothing stays quiet here too.
		if (state === "ready" && entry.preview?.from === "me") continue;
		const title = desk.name === undefined ? entry.persona.name : `${entry.persona.name} · ${desk.name}`;
		const target = desk.name === undefined ? entry.persona.id : `${desk.id}/${entry.persona.id}`;
		void postToast(target, title, lastLine(entry));
		if (state === "error") void requestAttention();
	}
	for (const key of lastState.keys()) {
		if (key.startsWith(scope) && !live.has(key)) lastState.delete(key);
		if (key.startsWith(scope) && !live.has(key)) lastPost.delete(key);
	}
}

/** Which desk and teammate a clicked toast is about; no desk means the one on screen. */
export function toastTarget(payload: string): { deskId: string | null; personaId: string } {
	const slash = payload.lastIndexOf("/");
	return slash === -1 ? { deskId: null, personaId: payload } : { deskId: payload.slice(0, slash), personaId: payload.slice(slash + 1) };
}

/** The chrome names who is open, so the task bar is the rail's selected row. */
function windowTitle(name: string | null): string {
	return name === null ? "Hotline" : `${name} — Hotline`;
}

export function setWindowTitle(name: string | null): void {
	const title = windowTitle(name);
	document.title = title;
	// The shell names the window: on macOS a new title puts the traffic
	// lights back at AppKit's default, and it seats them again. A browser
	// tab is not the desk; the document title is enough there.
	invoke("window_title", { title }).catch(() => {});
}

function lastLine(entry: RosterEntry): string {
	return (entry.preview?.text ?? "").replace(/\s+/g, " ").trim();
}
