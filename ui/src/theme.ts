import { useSyncExternalStore } from "react";
import { setNativeTheme } from "./native";

/**
 * Which palette the window draws in: the system's, or one the person picked
 * in Settings. It is this window's preference, not the room's, so it lives in
 * localStorage beside the rail's, and the phone keeps its own.
 *
 * tokens.css holds both palettes; this module only says which one by setting
 * `data-theme` on <html> to "light" or "dark". System follows the OS as it
 * changes, without a reload.
 */
export type Theme = "system" | "light" | "dark";

export const THEMES: { id: Theme; name: string }[] = [
	{ id: "system", name: "System" },
	{ id: "light", name: "Light" },
	{ id: "dark", name: "Dark" },
];

const KEY = "hotline.theme";
const SYSTEM_LIGHT = window.matchMedia("(prefers-color-scheme: light)");
const listeners = new Set<() => void>();

function stored(): Theme {
	const raw = localStorage.getItem(KEY);
	return raw === "light" || raw === "dark" ? raw : "system";
}

let current: Theme = stored();

function apply() {
	const light = current === "light" || (current === "system" && SYSTEM_LIGHT.matches);
	document.documentElement.dataset.theme = light ? "light" : "dark";
}

/** The window's own chrome follows the choice too, or the system's on System. */
function pinChrome() {
	void setNativeTheme(current === "system" ? null : current);
}

/** Draws the stored choice. Called once, before the first render, so the window never flashes the other palette. */
export function startTheme() {
	apply();
	pinChrome();
	SYSTEM_LIGHT.addEventListener("change", () => {
		if (current === "system") apply();
	});
	// Another window of the same app changed it.
	window.addEventListener("storage", (event) => {
		if (event.key !== KEY) return;
		current = stored();
		apply();
		for (const listener of listeners) listener();
	});
}

export function setTheme(theme: Theme) {
	current = theme;
	if (theme === "system") localStorage.removeItem(KEY);
	else localStorage.setItem(KEY, theme);
	apply();
	pinChrome();
	for (const listener of listeners) listener();
}

export function useTheme(): Theme {
	return useSyncExternalStore(
		(listener) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		() => current,
	);
}
