import { invoke } from "@tauri-apps/api/core";

/**
 * A teammate's chat opened in a window of its own, beside the main one
 * (chat_window.rs). The shell tells the page which teammate and desk it is
 * for. Such a window shows that conversation and nothing room-wide: no
 * rail, no toasts or dock badge, no global hotkeys or menu bar actions,
 * which the main window keeps. What it opens is its own and not saved, so
 * the main window still opens where it was.
 */
declare global {
	interface Window {
		__hotlineChat?: { deskId: string; personaId: string };
	}
}

/** This window's teammate and desk, or null in the main window. */
export const chatWindow: { deskId: string; personaId: string } | null = typeof window === "undefined" ? null : (window.__hotlineChat ?? null);

/** Opens the teammate's chat in a window of its own, or brings it forward when one is open. */
export async function openChatWindow(deskId: string, personaId: string, name: string): Promise<void> {
	await invoke("chat_window", { deskId, personaId, name });
}
