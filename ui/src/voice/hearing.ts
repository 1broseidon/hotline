import { useSyncExternalStore } from "react";

/**
 * Whether calls from this Mac hear the person on this Mac or send their
 * audio to the desk's transcription, as every other device does. It is this
 * computer's choice, not the room's, so it is kept here rather than in the
 * room's settings; on unless the person picked a provider instead.
 * Dictation always hears here: it has no other way yet.
 */
const KEY = "hotline.hearOnThisMac";
const listeners = new Set<() => void>();

export function hearOnThisMac(): boolean {
	try {
		return localStorage.getItem(KEY) !== "off";
	} catch {
		return true;
	}
}

export function setHearOnThisMac(on: boolean): void {
	try {
		localStorage.setItem(KEY, on ? "on" : "off");
	} catch {
		// Private storage refused: the choice lasts as long as the window.
	}
	for (const listener of listeners) listener();
}

export function subscribeHearing(listener: () => void): () => void {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useHearOnThisMac(): boolean {
	return useSyncExternalStore(subscribeHearing, hearOnThisMac, () => true);
}
