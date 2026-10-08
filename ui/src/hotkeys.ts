import { useSyncExternalStore } from "react";
import { register, unregister, unregisterAll } from "@tauri-apps/plugin-global-shortcut";
import { isDesktop, platform } from "./native";

/**
 * Shortcuts heard anywhere on this computer, not only in the window: the
 * system hands the keys to Hotline whichever app is in front, the shell
 * brings the window forward (hotline-app's global-shortcut handler), and
 * the window acts. They are this computer's, not the room's, so they live
 * in localStorage, and each person picks their own keys in Settings ›
 * General. Stored in the plugin's own words ("Control+Alt+KeyH"); an empty
 * string is a shortcut turned off.
 */

export type HotkeyId = "dictate" | "call";

export const HOTKEYS: readonly { id: HotkeyId; label: string; fallback: string }[] = [
	{ id: "dictate", label: "Dictate", fallback: "Control+Alt+KeyH" },
	{ id: "call", label: "Call", fallback: "" },
];

type Hotkeys = Record<HotkeyId, string>;

const KEY = "hotline.hotkeys";
const MAC = platform() === "macos";
const listeners = new Set<() => void>();

function stored(): Hotkeys {
	const keys = Object.fromEntries(HOTKEYS.map((hotkey) => [hotkey.id, hotkey.fallback])) as Hotkeys;
	try {
		const raw: unknown = JSON.parse(localStorage.getItem(KEY) ?? "{}");
		if (typeof raw !== "object" || raw === null) return keys;
		for (const hotkey of HOTKEYS) {
			const value = (raw as Record<string, unknown>)[hotkey.id];
			if (typeof value === "string" && (value === "" || acceleratorKeyLabel(value) !== null)) keys[hotkey.id] = value;
		}
	} catch {
		// Unreadable or private mode: the defaults.
	}
	return keys;
}

let current: Hotkeys = stored();

export function hotkeys(): Hotkeys {
	return current;
}

export function setHotkey(id: HotkeyId, accelerator: string): void {
	current = { ...current, [id]: accelerator };
	try {
		localStorage.setItem(KEY, JSON.stringify(current));
	} catch {
		// Private mode: the keys hold until the app quits.
	}
	for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useHotkeys(): Hotkeys {
	return useSyncExternalStore(subscribe, () => current);
}

// ------------------------------------------------------------- reading keys

const MODIFIERS = ["Control", "Alt", "Shift", "Super"] as const;
type Modifier = (typeof MODIFIERS)[number];

/** In the order each system writes them: ⌃⌥⇧⌘ on a Mac, Ctrl+Alt+Shift+Win elsewhere. */
const MAC_GLYPHS: Record<Modifier, string> = { Control: "⌃", Alt: "⌥", Shift: "⇧", Super: "⌘" };
const NAMES: Record<Modifier, string> = { Control: "Ctrl", Alt: "Alt", Shift: "Shift", Super: platform() === "windows" ? "Win" : "Super" };

const PUNCTUATION: Record<string, string> = {
	Backquote: "`",
	Backslash: "\\",
	BracketLeft: "[",
	BracketRight: "]",
	Comma: ",",
	Equal: "=",
	Minus: "-",
	Period: ".",
	Quote: "'",
	Semicolon: ";",
	Slash: "/",
};

const NAMED: Record<string, string> = {
	Space: "Space",
	Enter: MAC ? "Return" : "Enter",
	Tab: "Tab",
	Backspace: MAC ? "Delete" : "Backspace",
	Delete: MAC ? "Fwd Del" : "Delete",
	Home: "Home",
	End: "End",
	PageUp: "Page Up",
	PageDown: "Page Down",
	ArrowUp: "↑",
	ArrowDown: "↓",
	ArrowLeft: "←",
	ArrowRight: "→",
};

/** How a key the system can bind reads on its cap, or null for a key it cannot (a modifier alone, Escape, an IME key). */
function keyLabel(code: string): string | null {
	const letter = /^Key([A-Z])$/.exec(code);
	if (letter) return letter[1]!;
	const digit = /^Digit([0-9])$/.exec(code);
	if (digit) return digit[1]!;
	const numpad = /^Numpad([0-9])$/.exec(code);
	if (numpad) return `Num ${numpad[1]!}`;
	if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
	return PUNCTUATION[code] ?? NAMED[code] ?? null;
}

/** The key's cap for an accelerator, or null when it is not one this window writes. */
function acceleratorKeyLabel(accelerator: string): string | null {
	const parts = accelerator.split("+");
	const key = parts.pop() ?? "";
	if (parts.length === 0 || !parts.every((part) => (MODIFIERS as readonly string[]).includes(part))) return null;
	return keyLabel(key);
}

/** ⌃⌥H on a Mac, Ctrl+Alt+H elsewhere; empty for a shortcut that is off. */
export function hotkeyLabel(accelerator: string, mac = MAC): string {
	const key = acceleratorKeyLabel(accelerator);
	if (key === null) return "";
	const held = MODIFIERS.filter((modifier) => accelerator.split("+").includes(modifier));
	if (mac) return `${held.map((modifier) => MAC_GLYPHS[modifier]).join("")}${key}`;
	return [...held.map((modifier) => NAMES[modifier]), key].join("+");
}

type KeyPress = Pick<KeyboardEvent, "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey">;

/**
 * What a press in the key recorder binds: at least one modifier and a key
 * the system can bind, or null while only modifiers are held. Shift alone
 * is not enough, because Shift and a letter is typing.
 */
export function hotkeyFromPress(press: KeyPress): string | null {
	const held: Modifier[] = [];
	if (press.ctrlKey) held.push("Control");
	if (press.altKey) held.push("Alt");
	if (press.shiftKey) held.push("Shift");
	if (press.metaKey) held.push("Super");
	if (held.length === 0 || (held.length === 1 && held[0] === "Shift")) return null;
	if (keyLabel(press.code) === null) return null;
	return [...held, press.code].join("+");
}

// ------------------------------------------------------------- registering

/** The plugin, or a fake in tests. */
export type ShortcutPlugin = {
	register(accelerator: string, onPress: () => void): Promise<void>;
	unregister(accelerator: string): Promise<void>;
	unregisterAll(): Promise<void>;
};

const tauriShortcuts: ShortcutPlugin = {
	register: (accelerator, onPress) =>
		register(accelerator, (event) => {
			if (event.state === "Pressed") onPress();
		}),
	unregister: (accelerator) => unregister(accelerator),
	unregisterAll: () => unregisterAll(),
};

/**
 * Keeps the system's registrations equal to what is wanted, one change at a
 * time, and remembers which keys the system refused. A page that reloads
 * finds its old registrations still held by the shell, so the first sync
 * lets every one of them go before taking any.
 */
export class HotkeyRegistrar {
	private held: Partial<Record<HotkeyId, string>> = {};
	private refusals: Partial<Record<HotkeyId, string>> = {};
	private queue: Promise<void>;
	private readonly listeners = new Set<() => void>();

	constructor(
		private readonly plugin: ShortcutPlugin,
		private readonly onPress: (id: HotkeyId) => void,
	) {
		this.queue = plugin.unregisterAll().catch(() => {});
	}

	/** Why the system refused a shortcut's keys, by shortcut. */
	get refused(): Partial<Record<HotkeyId, string>> {
		return this.refusals;
	}

	readonly watch = (listener: () => void): (() => void) => {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	};

	/** Settles after this and every earlier change has reached the system. */
	sync(wanted: Partial<Record<HotkeyId, string>>): Promise<void> {
		this.queue = this.queue.then(() => this.apply(wanted));
		return this.queue;
	}

	private async apply(wanted: Partial<Record<HotkeyId, string>>): Promise<void> {
		const refusals = { ...this.refusals };
		for (const { id } of HOTKEYS) {
			const want = wanted[id] ?? "";
			// Only keys the system took are held, so keys it refused are asked for again.
			const have = this.held[id] ?? "";
			delete refusals[id];
			if (want === have) continue;
			if (have !== "") {
				await this.plugin.unregister(have).catch(() => {});
				delete this.held[id];
			}
			if (want === "") continue;
			try {
				await this.plugin.register(want, () => this.onPress(id));
				this.held[id] = want;
			} catch {
				refusals[id] = refusalText(want);
			}
		}
		this.refusals = refusals;
		for (const listener of this.listeners) listener();
	}
}

/** The system says little more than no, and the usual reason is another app holding the keys. */
function refusalText(accelerator: string): string {
	return `Hotline couldn't take ${hotkeyLabel(accelerator)}. Another app may be using it; pick other keys.`;
}

/** What a press does: the window that is up says, and a desk switched to says again. */
let pressed: ((id: HotkeyId) => void) | null = null;
export function onHotkey(handler: (id: HotkeyId) => void): () => void {
	pressed = handler;
	return () => {
		if (pressed === handler) pressed = null;
	};
}

/** The window's registrar; none in a browser tab, which cannot hear keys outside itself. */
let registrar: HotkeyRegistrar | null | undefined;
export function hotkeyRegistrar(): HotkeyRegistrar | null {
	if (registrar === undefined) registrar = isDesktop() ? new HotkeyRegistrar(tauriShortcuts, (id) => pressed?.(id)) : null;
	return registrar;
}

/** Why the system refused each shortcut, for Settings. */
export function useHotkeyRefusals(): Partial<Record<HotkeyId, string>> {
	return useSyncExternalStore(
		(listener) => registrar?.watch(listener) ?? (() => {}),
		() => registrar?.refused ?? NONE,
	);
}
const NONE: Partial<Record<HotkeyId, string>> = {};

/**
 * While Settings records new keys the shortcuts are let go, or pressing the
 * keys already bound would do their job instead of reaching the recorder.
 */
let recording = false;
export function setRecording(on: boolean): void {
	recording = on;
	for (const listener of listeners) listener();
}
export function useRecording(): boolean {
	return useSyncExternalStore(subscribe, () => recording);
}
