/**
 * Desktop toasts for the two moments that earn one: a turn ending, and a
 * teammate blocked.
 *
 * The judgement lives here, not in the shell, because the roster view already
 * carries the session edge and the last line, and a toast about the screen
 * already in your hand is noise. The plugin posts; this file only decides.
 * Phone push is a later seat — not this.
 */

import { getCurrentWindow } from "@tauri-apps/api/window";
import {
	isPermissionGranted,
	onAction,
	requestPermission,
	sendNotification,
} from "@tauri-apps/plugin-notification";
import type { SessionState } from "./generated/contract";
import type { RosterEntry } from "./wire";

/** Last state seen per teammate, so a transition can be recognised as one. */
const lastState = new Map<string, SessionState>();

/**
 * A roster fold just arrived. Notify only on thinking → ready or thinking →
 * error, and only when this window is not focused. A snapshot, a teammate
 * that was never thinking, and a window you are looking at are all silent.
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
		void tell(entry.persona.name, lastLine(entry));
	}
	for (const id of lastState.keys()) {
		if (!live.has(id)) lastState.delete(id);
	}
}

/**
 * Clicking a toast should raise this window. The plugin's action channel is
 * mobile-only today; on the desk the OS may still raise us, and a missing
 * channel is nothing rather than a second path.
 */
export function watchNotificationClicks(): () => void {
	let stop: (() => void) | undefined;
	void onAction(() => {
		try {
			void getCurrentWindow().setFocus();
		} catch {
			// A browser tab has no window to raise.
		}
	})
		.then((listener) => {
			stop = () => {
				void listener.unregister();
			};
		})
		.catch(() => {});
	return () => stop?.();
}

/** The chrome names who is open, so the task bar is the rail's selected row. */
export function windowTitle(name: string | null): string {
	return name === null ? "Toad" : `${name} — Toad`;
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

async function tell(title: string, body: string): Promise<void> {
	try {
		let granted = await isPermissionGranted();
		if (!granted) granted = (await requestPermission()) === "granted";
		if (!granted) return;
		sendNotification({ title, body });
	} catch {
		// A missed toast is not a failed turn.
	}
}
