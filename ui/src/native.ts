/**
 * The desk's native pieces: a folder picker, opening a path or a link, the
 * clipboard, the menu the shell emits, the window chrome, the dock, toasts,
 * and the version and data directory the shell injected. Each call is a no-op — or a web
 * fallback — in a browser tab, so the window can still typecheck and render
 * there.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Menu } from "@tauri-apps/api/menu";
import { UserAttentionType, getCurrentWindow } from "@tauri-apps/api/window";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";

export function isDesktop(): boolean {
	return window.__toadDesk !== undefined;
}

export function platform(): string {
	return window.__toadDesk?.platform ?? "web";
}

/** Whether the page draws the window's frame. macOS keeps its traffic lights. */
export function drawsFrame(): boolean {
	const os = platform();
	return os === "linux" || os === "windows";
}

export function appVersion(): string {
	return window.__toadDesk?.version ?? "";
}

export function dataDirectory(): string {
	return window.__toadDesk?.dataDir ?? "";
}

export async function pickDirectory(): Promise<string | null> {
	try {
		const selected = await open({ directory: true, multiple: false });
		return typeof selected === "string" ? selected : null;
	} catch {
		return null;
	}
}

/** Files chosen to ride with a message; none when the picker was dismissed. */
export async function pickFiles(): Promise<string[]> {
	try {
		const selected = await open({ multiple: true });
		return Array.isArray(selected) ? selected : typeof selected === "string" ? [selected] : [];
	} catch {
		return [];
	}
}

export async function openLink(href: string): Promise<void> {
	try {
		await openUrl(href);
	} catch {
		window.open(href, "_blank", "noopener,noreferrer");
	}
}

export async function revealPath(path: string): Promise<void> {
	if (path === "") return;
	try {
		await revealItemInDir(path);
	} catch {
		// A browser tab has no finder.
	}
}

export async function writeClipboard(text: string): Promise<void> {
	try {
		await writeText(text);
	} catch {
		await navigator.clipboard.writeText(text);
	}
}

export async function confirmRemove(name: string): Promise<boolean> {
	try {
		return await ask(`Remove ${name}? Their conversation goes too.`, {
			title: "Toad",
			kind: "warning",
		});
	} catch {
		return false;
	}
}

export async function toggleMaximize(): Promise<void> {
	try {
		await getCurrentWindow().toggleMaximize();
	} catch {
		// A browser tab is not a window.
	}
}

export async function minimizeWindow(): Promise<void> {
	try {
		await getCurrentWindow().minimize();
	} catch {
		// A browser tab is not a window.
	}
}

export async function closeWindow(): Promise<void> {
	try {
		await getCurrentWindow().close();
	} catch {
		// A browser tab is not a window.
	}
}

/**
 * The dock's badge: how many teammates have said something not yet read.
 * Nothing at zero, not a zero.
 */
export async function setBadge(count: number): Promise<void> {
	try {
		await getCurrentWindow().setBadgeCount(count > 0 ? count : undefined);
	} catch {
		// A browser tab has no dock.
	}
}

/** One bounce of the dock icon, or a flash of the taskbar button, for a window not in focus. */
export async function requestAttention(): Promise<void> {
	try {
		await getCurrentWindow().requestUserAttention(UserAttentionType.Informational);
	} catch {
		// A browser tab has no dock.
	}
}

/**
 * A desktop toast for a teammate. On macOS the shell posts it through the
 * notification center, which threads toasts by teammate and hands a click
 * back (`listenToastClicks`); elsewhere the plugin posts it, and a click is
 * the OS's own business. The first toast asks the person's leave. Nothing
 * from a browser tab, and nothing from a macOS `make dev`, which is a bare
 * binary the center will not post for.
 */
export async function postToast(personaId: string, title: string, body: string): Promise<void> {
	try {
		if (platform() === "macos") {
			await invoke("notify", { personaId, title, body });
			return;
		}
		let granted = await isPermissionGranted();
		if (!granted) granted = (await requestPermission()) === "granted";
		if (granted) sendNotification({ title, body });
	} catch {
		// A missed toast is not a failed turn.
	}
}

/** A toast was clicked: the shell has raised the window, and this is whose toast it was. */
export function listenToastClicks(onPersona: (personaId: string) => void): () => void {
	let stop: (() => void) | undefined;
	void listen<string>("toad://notification", (event) => {
		onPersona(event.payload);
	})
		.then((unlisten) => {
			stop = unlisten;
		})
		.catch(() => {});
	return () => stop?.();
}

export type WindowShape = { maximized: boolean; fullscreen: boolean };

/** Tells `onChange` the window's shape, now and on every resize. */
export function watchWindowShape(onChange: (shape: WindowShape) => void): () => void {
	let stop: (() => void) | undefined;
	let gone = false;
	const current = getCurrentWindow();
	const read = () => {
		Promise.all([current.isMaximized(), current.isFullscreen()])
			.then(([maximized, fullscreen]) => {
				if (!gone) onChange({ maximized, fullscreen });
			})
			.catch(() => {});
	};
	read();
	current
		.onResized(read)
		.then((unlisten) => {
			if (gone) unlisten();
			else stop = unlisten;
		})
		.catch(() => {});
	return () => {
		gone = true;
		stop?.();
	};
}

export function listenMenu(onAction: (id: string) => void): () => void {
	let stop: (() => void) | undefined;
	void listen<string>("toad://menu", (event) => {
		onAction(event.payload);
	})
		.then((unlisten) => {
			stop = unlisten;
		})
		.catch(() => {});
	return () => stop?.();
}

export async function popupTeammateMenu(actions: {
	onOpen(): void;
	onEdit(): void;
	onDelete(): void;
}): Promise<void> {
	try {
		const menu = await Menu.new({
			items: [
				{ id: "open", text: "Open", action: actions.onOpen },
				{ id: "edit", text: "Edit", action: actions.onEdit },
				{ id: "delete", text: "Delete", action: actions.onDelete },
			],
		});
		await menu.popup();
	} catch {
		// A browser tab has the page menu.
	}
}
