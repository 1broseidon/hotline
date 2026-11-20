import { useSyncExternalStore } from "react";

/**
 * Whether calls from this Mac hear the person on this Mac or send their
 * audio to the desk's transcription, as every other device does. It is this
 * computer's choice, not the room's, so it is kept here rather than in the
 * room's settings; on unless the person picked a provider instead.
 * Dictation hears here too, unless the person picked something else and
 * the desk has a model of its own to hear with (dictation.ts).
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

/**
 * Whether a Turn by turn call heard by this Mac can be interrupted by
 * talking over the teammate (bargeIn.ts), rather than only by a tap. Off
 * unless the person turned it on: it relies on this Mac's echo cancellation,
 * which a headset makes easy and open speakers may defeat, and only the
 * person knows which they have. Kept here for the same reason as hearing.
 */
const INTERRUPT_KEY = "hotline.interruptBySpeaking";

export function interruptBySpeaking(): boolean {
	try {
		return localStorage.getItem(INTERRUPT_KEY) === "on";
	} catch {
		return false;
	}
}

export function setInterruptBySpeaking(on: boolean): void {
	try {
		localStorage.setItem(INTERRUPT_KEY, on ? "on" : "off");
	} catch {
		// Private storage refused: the choice lasts as long as the window.
	}
	for (const listener of listeners) listener();
}

export function useInterruptBySpeaking(): boolean {
	return useSyncExternalStore(subscribeHearing, interruptBySpeaking, () => false);
}
