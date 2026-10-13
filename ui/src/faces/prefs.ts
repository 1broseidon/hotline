import { useSyncExternalStore } from "react";

/**
 * How this window shows teammates' faces. Like the theme it is the person's
 * preference, not the room's, so it lives in localStorage and a phone keeps
 * its own. Off draws every teammate exactly as before: its picture or its
 * initial. Which face a teammate has is its look, on the desk.
 */
export type FacePrefs = {
	on: boolean;
	/** The large face above a conversation. */
	stage: boolean;
	/** The face in place of the activity mark under the transcript. */
	mark: boolean;
	/** Asleep while the window is in the background. */
	doze: boolean;
};

const KEY = "hotline.faces";
const DEFAULTS: FacePrefs = { on: false, stage: true, mark: true, doze: true };
const listeners = new Set<() => void>();

function stored(): FacePrefs {
	try {
		const raw = JSON.parse(localStorage.getItem(KEY) ?? "null") as Partial<FacePrefs> | null;
		return { ...DEFAULTS, ...(raw ?? {}) };
	} catch {
		return DEFAULTS;
	}
}

let current: FacePrefs | undefined;

function read(): FacePrefs {
	current ??= stored();
	return current;
}

export function setFacePrefs(patch: Partial<FacePrefs>) {
	current = { ...read(), ...patch };
	localStorage.setItem(KEY, JSON.stringify(current));
	for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useFacePrefs(): FacePrefs {
	return useSyncExternalStore(subscribe, read, read);
}
