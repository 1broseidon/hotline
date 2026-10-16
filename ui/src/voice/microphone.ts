import { platform } from "../native";

/**
 * Why the microphone would not open, said for the computer it is on, from
 * the DOMException `getUserMedia` rejects with: none there, busy (another
 * app has it, as on a call elsewhere), or refused, where the way to allow
 * it is where that system keeps it. Dictation and calls say the same.
 */
export const NO_MICROPHONE = "No microphone found. Connect one or pick it in your computer's sound settings, then try again.";
export const MICROPHONE_BUSY = "The microphone is busy or not working. If another app is using it, close it, then try again.";

/** Refused: where this system turns the microphone on for an app. */
export function microphoneDenied(os: string = platform()): string {
	switch (os) {
		case "macos":
			return "Hotline can't use the microphone. Turn it on for Hotline in System Settings › Privacy & Security › Microphone, then try again.";
		case "windows":
			return "Hotline can't use the microphone. Turn on microphone access for apps in Settings › Privacy & security › Microphone, then try again.";
		default:
			return "Hotline can't use the microphone. Check it is allowed and picked in your sound settings, then try again.";
	}
}

export function microphoneTrouble(error: unknown, os: string = platform()): string {
	const name = typeof error === "object" && error !== null && "name" in error ? error.name : "";
	if (name === "NotFoundError" || name === "OverconstrainedError") return NO_MICROPHONE;
	if (name === "NotReadableError" || name === "AbortError") return MICROPHONE_BUSY;
	return microphoneDenied(os);
}
