/**
 * Every chord the window hears. The keydown handler matches `match`; Help
 * lists every row; titles read `keys`. One table, so a shortcut the pane
 * forgot or the handler invented is not a shortcut.
 */

export const CHORD_GROUPS = [
	{ id: "room", title: "Room" },
	{ id: "conversation", title: "Conversation" },
	{ id: "panes", title: "Panes" },
] as const;

export type ChordGroup = (typeof CHORD_GROUPS)[number]["id"];

/** A letter or comma under Ctrl, a digit seat, or Escape with no modifier. */
export type ChordMatch =
	| { ctrl: true; key: string; code?: string }
	| { ctrl: true; digit: true }
	| { key: "Escape" };

export type Chord = {
	id: string;
	group: ChordGroup;
	label: string;
	/** Shown on Help and in titles. Always Ctrl, matching the listener. */
	keys: string;
	/** What the window listener compares. A field-owned chord has none. */
	match?: ChordMatch;
};

export const CHORDS: readonly Chord[] = [
	{ id: "new-teammate", group: "room", label: "New teammate", keys: "Ctrl+N", match: { ctrl: true, key: "n", code: "KeyN" } },
	{ id: "settings", group: "room", label: "Settings", keys: "Ctrl+,", match: { ctrl: true, key: ",", code: "Comma" } },
	{
		id: "teammate-seat",
		group: "room",
		label: "Teammate 1–9",
		keys: "Ctrl+1–9",
		match: { ctrl: true, digit: true },
	},
	{ id: "search", group: "conversation", label: "Search", keys: "Ctrl+F", match: { ctrl: true, key: "f", code: "KeyF" } },
	{ id: "teammate", group: "conversation", label: "Teammate", keys: "Ctrl+I", match: { ctrl: true, key: "i", code: "KeyI" } },
	{ id: "reply", group: "conversation", label: "Reply", keys: "R" },
	{ id: "send", group: "conversation", label: "Send", keys: "Enter" },
	{ id: "newline", group: "conversation", label: "New line", keys: "Shift+Enter" },
	{ id: "interrupt", group: "conversation", label: "Interrupt", keys: "Esc" },
	{ id: "close", group: "panes", label: "Close", keys: "Esc", match: { key: "Escape" } },
];

export function chordKeys(id: string): string {
	return CHORDS.find((chord) => chord.id === id)?.keys ?? "";
}

/** The compact mark the overflow menu already used: ⌃I, not Ctrl+I. */
export function chordGlyph(id: string): string {
	const keys = chordKeys(id);
	return keys.startsWith("Ctrl+") ? `\u2303${keys.slice("Ctrl+".length)}` : keys;
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
		if (!heldCtrl(event)) return null;
		const seat = Number(event.key);
		if (!Number.isInteger(seat) || seat < 1 || seat > 9) return null;
		return `teammate-${seat}`;
	}
	if ("ctrl" in wanted) {
		if (!heldCtrl(event)) return null;
		if (event.key === wanted.key || (wanted.code !== undefined && event.code === wanted.code)) {
			return chord.id;
		}
		return null;
	}
	return event.key === "Escape" ? chord.id : null;
}

function heldCtrl(event: KeyboardEvent): boolean {
	return event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey;
}
