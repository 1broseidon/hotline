/**
 * Every chord the window hears. The keydown handler matches `match`; Help
 * lists every row; titles read `keys`. One table, so a shortcut the pane
 * forgot or the handler invented is not a shortcut.
 *
 * The chord key is Cmd on a Mac and Ctrl everywhere else, the way each
 * platform's own apps are: a Mac hand reaches for Cmd+, without thinking,
 * and Ctrl+N in a Mac text field is "next line". The menu bar's
 * accelerators (toad-desktop) say the same.
 */

import { platform } from "./native";

const MAC = platform() === "macos";
const MOD = MAC ? "\u2318" : "Ctrl";

function mod(key: string): string {
	return `${MOD}+${key}`;
}

export const CHORD_GROUPS = [
	{ id: "room", title: "Room" },
	{ id: "conversation", title: "Conversation" },
	{ id: "panes", title: "Panes" },
] as const;

export type ChordGroup = (typeof CHORD_GROUPS)[number]["id"];

/** A letter or comma under the chord key, a digit seat, or Escape with no modifier. */
export type ChordMatch =
	| { mod: true; key: string; code?: string }
	| { mod: true; digit: true }
	| { key: "Escape" };

export type Chord = {
	id: string;
	group: ChordGroup;
	label: string;
	/** Shown on Help and in titles. The platform's chord key, matching the listener. */
	keys: string;
	/** What the window listener compares. A field-owned chord has none. */
	match?: ChordMatch;
};

export const CHORDS: readonly Chord[] = [
	{ id: "new-teammate", group: "room", label: "New teammate", keys: mod("N"), match: { mod: true, key: "n", code: "KeyN" } },
	{ id: "settings", group: "room", label: "Settings", keys: mod(","), match: { mod: true, key: ",", code: "Comma" } },
	{
		id: "teammate-seat",
		group: "room",
		label: "Teammate 1–9",
		keys: mod("1–9"),
		match: { mod: true, digit: true },
	},
	{ id: "search", group: "conversation", label: "Search", keys: mod("F"), match: { mod: true, key: "f", code: "KeyF" } },
	{ id: "teammate", group: "conversation", label: "Teammate", keys: mod("I"), match: { mod: true, key: "i", code: "KeyI" } },
	{ id: "reply", group: "conversation", label: "Reply", keys: "R" },
	{ id: "send", group: "conversation", label: "Send", keys: "Enter" },
	{ id: "newline", group: "conversation", label: "New line", keys: "Shift+Enter" },
	{ id: "interrupt", group: "conversation", label: "Interrupt", keys: "Esc" },
	{ id: "close", group: "panes", label: "Close", keys: "Esc", match: { key: "Escape" } },
];

export function chordKeys(id: string): string {
	return CHORDS.find((chord) => chord.id === id)?.keys ?? "";
}

/** The compact mark the overflow menu uses: ⌘I on a Mac, ⌃I elsewhere, not Ctrl+I. */
export function chordGlyph(id: string): string {
	const keys = chordKeys(id);
	const prefix = `${MOD}+`;
	if (!keys.startsWith(prefix)) return keys;
	return `${MAC ? "\u2318" : "\u2303"}${keys.slice(prefix.length)}`;
}

/**
 * Which row this key is, or null. Digit seats return `teammate-1`…`teammate-9`
 * so the handler can pick a rail slot without a second table.
 */
export function matchChord(event: KeyboardEvent): string | null {
	for (const chord of CHORDS) {
		const hit = matches(chord, event);
		if (hit !== null) return hit;
	}
	return null;
}

function matches(chord: Chord, event: KeyboardEvent): string | null {
	const wanted = chord.match;
	if (wanted === undefined) return null;
	if ("digit" in wanted) {
		if (!heldMod(event)) return null;
		const seat = Number(event.key);
		if (!Number.isInteger(seat) || seat < 1 || seat > 9) return null;
		return `teammate-${seat}`;
	}
	if ("mod" in wanted) {
		if (!heldMod(event)) return null;
		if (event.key === wanted.key || (wanted.code !== undefined && event.code === wanted.code)) {
			return chord.id;
		}
		return null;
	}
	return event.key === "Escape" ? chord.id : null;
}

function heldMod(event: KeyboardEvent): boolean {
	const held = MAC ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
	return held && !event.altKey && !event.shiftKey;
}
