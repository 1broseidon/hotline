/**
 * The desk's native pieces: a folder picker, opening a path or a link, the
 * clipboard, the menu the shell emits, the window chrome, the dock, toasts,
 * and the version and data directory the shell injected. Each call is a no-op — or a web
 * fallback — in a browser tab, so the window can still typecheck and render
 * there.
 */

import { setTheme as setAppTheme } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Menu } from "@tauri-apps/api/menu";
import { UserAttentionType, getCurrentWindow } from "@tauri-apps/api/window";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";

export function isDesktop(): boolean {
	return window.__hotlineDesk !== undefined;
}

export function platform(): string {
	return window.__hotlineDesk?.platform ?? "web";
}

/** Brings the window forward from wherever the person was: shown, restored and focused. */
export async function showWindow(): Promise<void> {
	if (!isDesktop()) return;
	const window = getCurrentWindow();
	await window.show();
	await window.unminimize();
	await window.setFocus();
}

/** Whether the page draws the window's frame. macOS keeps its traffic lights. */
export function drawsFrame(): boolean {
	const os = platform();
	return os === "linux" || os === "windows";
}

export function appVersion(): string {
	return window.__hotlineDesk?.version ?? "";
}

export function dataDirectory(): string {
	return window.__hotlineDesk?.dataDir ?? "";
}

/** The computer image this build of Hotline pins, so a blank field can show it. */
export function pinnedComputerImage(): string {
	return window.__hotlineDesk?.computerImage ?? "";
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
			title: "Hotline",
			kind: "warning",
		});
	} catch {
		return false;
	}
}

export async function confirmRemovePicture(name: string): Promise<boolean> {
	try {
		return await ask(`Remove ${name}'s picture? Their initial takes its place.`, {
			title: "Hotline",
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

/**
 * The theme the shell draws the windows' own chrome in (the title bar, the
 * frame, native menus), so it matches the palette picked in Settings. Null
 * hands it back to the system.
 */
export async function setNativeTheme(theme: "light" | "dark" | null): Promise<void> {
	try {
		await setAppTheme(theme);
	} catch {
		// A browser tab has no chrome of its own.
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
	void listen<string>("hotline://notification", (event) => {
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
	let settle: ReturnType<typeof setTimeout> | undefined;
	const read = () => {
		Promise.all([current.isMaximized(), current.isFullscreen()])
			.then(([maximized, fullscreen]) => {
				if (!gone) onChange({ maximized, fullscreen });
			})
			.catch(() => {});
	};
	read();
	// A drag of the window edge is a resize every frame, and each read is two
	// round trips to the shell; the shape is only asked once it holds still.
	const later = () => {
		clearTimeout(settle);
		settle = setTimeout(read, 150);
	};
	current
		.onResized(later)
		.then((unlisten) => {
			if (gone) unlisten();
			else stop = unlisten;
		})
		.catch(() => {});
	return () => {
		gone = true;
		clearTimeout(settle);
		stop?.();
	};
}

export function listenMenu(onAction: (id: string) => void): () => void {
	let stop: (() => void) | undefined;
	void listen<string>("hotline://menu", (event) => {
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
	/** Pin or unpin, and while pinned, move along the row. Pin is disabled when the desk has its three. */
	pin: { pinned: boolean; full: boolean; onToggle(): void; onLeft?(): void; onRight?(): void };
}): Promise<void> {
	try {
		const { pin } = actions;
		const menu = await Menu.new({
			items: [
				{ id: "open", text: "Open", action: actions.onOpen },
				{ id: "edit", text: "Edit", action: actions.onEdit },
				{ item: "Separator" as const },
				{ id: "pin", text: pin.pinned ? "Unpin" : "Pin to top", enabled: pin.pinned || !pin.full, action: pin.onToggle },
				...(pin.onLeft !== undefined ? [{ id: "pin-left", text: "Move left", action: pin.onLeft }] : []),
				...(pin.onRight !== undefined ? [{ id: "pin-right", text: "Move right", action: pin.onRight }] : []),
				{ item: "Separator" as const },
				{ id: "delete", text: "Delete", action: actions.onDelete },
			],
		});
		await menu.popup();
	} catch {
		// A browser tab has the page menu.
	}
}

/**
 * A bubble's own menu, for a right-click with nothing selected: what the
 * hover bar offers, as the platform's menu. React is a submenu of the same
 * six the phone's long-press offers.
 */
export async function popupMessageMenu(actions: {
	reactions: readonly string[];
	onReact?(emoji: string): void;
	onReply?(): void;
	onCopy(): void;
}): Promise<void> {
	try {
		const { onReact, onReply } = actions;
		const menu = await Menu.new({
			items: [
				...(onReply !== undefined ? [{ id: "reply", text: "Reply", action: onReply }] : []),
				{ id: "copy", text: "Copy", action: actions.onCopy },
				...(onReact !== undefined
					? [
							{ item: "Separator" as const },
							{
								id: "react",
								text: "React",
								items: actions.reactions.map((emoji) => ({ id: `react:${emoji}`, text: emoji, action: () => onReact(emoji) })),
							},
						]
					: []),
			],
		});
		await menu.popup();
	} catch {
		// A browser tab has the page menu.
	}
}

export type UpdateStatus = {
	current: string;
	available: { version: string; notes: string } | null;
	checkedAt: number | null;
	phase: "idle" | "checking" | "downloading" | "installing" | "restarting";
	downloaded: number;
	total: number | null;
	error: string | null;
	disabledReason: string | null;
};

/**
 * A dev build never checks for updates, so `VITE_PREVIEW_UPDATE=0.35.1 make dev`
 * pretends that version is out: the corner card and Settings › Updates show
 * it, with notes in the changelog's shape. Installing it fails, as it should.
 */
export const PREVIEW_UPDATE = import.meta.env.DEV ? (import.meta.env.VITE_PREVIEW_UPDATE as string | undefined) : undefined;
const PREVIEW_NOTES = `### Added

- When a new version is out, a small card in the window's corner says so.

### Fixed

- Release notes in **Settings › Updates** read as formatted text, not markdown.
- An example fix with \`inline code\` and a [link](https://github.com/1broseidon/hotline/releases).
`;

export async function updateStatus(): Promise<UpdateStatus> {
	if (PREVIEW_UPDATE) return {
		current: appVersion(), available: { version: PREVIEW_UPDATE, notes: PREVIEW_NOTES }, checkedAt: Math.floor(Date.now() / 1000),
		phase: "idle", downloaded: 0, total: null, error: null, disabledReason: null,
	};
	if (!isDesktop()) return {
		current: appVersion(), available: null, checkedAt: null, phase: "idle",
		downloaded: 0, total: null, error: null,
		disabledReason: "Open the desktop application to check for updates.",
	};
	return invoke<UpdateStatus>("get_update_status");
}

/** Read after subscribing so reopening Settings catches up with a background check. */
export function watchUpdates(onChange: (status: UpdateStatus) => void, onError: (error: unknown) => void): () => void {
	let gone = false;
	let stop: (() => void) | undefined;
	void (async () => {
		try {
			if (isDesktop() && !PREVIEW_UPDATE) {
				const unlisten = await listen<UpdateStatus>("hotline://update", (event) => {
					if (!gone) onChange(event.payload);
				});
				if (gone) { unlisten(); return; }
				stop = unlisten;
			}
			const status = await updateStatus();
			if (!gone) onChange(status);
		} catch (error) { if (!gone) onError(error); }
	})();
	return () => { gone = true; stop?.(); };
}

export async function checkUpdate(): Promise<void> { await invoke("check_update"); }
export async function installUpdate(version: string): Promise<void> { await invoke("install_update", { version }); }
export async function cancelUpdate(): Promise<void> { await invoke("cancel_update"); }

/** Opens a PDF or a picture a teammate sent in the system's own viewer. The shell opens nothing else. */
export async function openSentFile(path: string): Promise<void> {
	if (!isDesktop()) throw new Error("Open the desktop application to open this.");
	await invoke("open_sent_file", { path });
}

/**
 * Saves a copy of a file a teammate sent where the person picks, in the
 * system's save dialog. Where the copy went, or null when it was dismissed.
 */
export async function saveSentFile(path: string): Promise<string | null> {
	if (!isDesktop()) throw new Error("Open the desktop application to save this.");
	return invoke<string | null>("save_sent_file", { path });
}

/**
 * Pairs this window with a remote desk from the link `hotline pair --link`
 * printed on the server, and answers the new desk's id. The shell claims
 * the invitation, keeps the keys, starts the desk's bridge and sends the
 * new desk list (see desks.ts).
 */
export async function pairDeskByLink(link: string): Promise<string> {
	return invoke<string>("desk_pair_link", { link });
}

/** The same, reading the payload by running `hotline pair --json` on `target` with the person's own `ssh`. */
export async function pairDeskOverSsh(target: string): Promise<string> {
	return invoke<string>("desk_pair_ssh", { target });
}

/** Unpairs a remote desk from this window: it forgets the keys and the desk leaves the list. */
export async function forgetDesk(deskId: string): Promise<void> {
	await invoke("desk_forget", { deskId });
}
