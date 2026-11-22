import { invoke } from "@tauri-apps/api/core";
import { isDesktop } from "../native";

/**
 * One `[voice]` line in the desk app's log (logs/hotline.log), beside the
 * desk's own, so a call's decisions taken in the window can be read with
 * the rest of it. Never a transcript: timings, counts and reasons only.
 * Outside the shell (a browser tab, a test) the line goes to the console.
 */
export function voiceNote(line: string): void {
	if (typeof window !== "undefined" && isDesktop()) {
		void invoke("voice_note", { line }).catch(() => console.info(`[voice] ${line}`));
		return;
	}
	console.info(`[voice] ${line}`);
}
