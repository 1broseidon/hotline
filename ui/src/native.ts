/**
 * The desk's native pieces: a folder picker, opening a path or a link, the
 * clipboard, the menu the shell emits, the window chrome, and the version
 * and data directory the shell injected. Each call is a no-op — or a web
 * fallback — in a browser tab, so the window can still typecheck and render
 * there.
 */

import { listen } from "@tauri-apps/api/event";
import { Menu } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";

export function isDesktop(): boolean {
	return window.__toadDesk !== undefined;
}

export function platform(): string {
	return window.__toadDesk?.platform ?? "web";
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
