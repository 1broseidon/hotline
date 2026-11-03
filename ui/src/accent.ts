/**
 * The accent is the person's, not the window's: the room keeps it, so every
 * window and the phone draw the same one (decision-42). tokens.css holds the
 * presets; this only sets `data-accent` on <html>, and none for mint, the
 * default. The last one seen is kept here too, so a window opens in it
 * instead of flashing mint until the room answers.
 */
export type Accent = "mint" | "teal" | "sky" | "iris" | "orchid" | "rose";

export const ACCENTS: { id: Accent; name: string }[] = [
	{ id: "mint", name: "Mint" },
	{ id: "teal", name: "Teal" },
	{ id: "sky", name: "Sky" },
	{ id: "iris", name: "Iris" },
	{ id: "orchid", name: "Orchid" },
	{ id: "rose", name: "Rose" },
];

const ACCENT_KEY = "hotline.accent";

/** A name the room sent, or none: anything this window doesn't know draws as mint. */
export function accentOf(name: string | null | undefined): Accent {
	return ACCENTS.some((accent) => accent.id === name) ? (name as Accent) : "mint";
}

/** Draws an accent; called with the room's choice whenever it arrives or changes. */
export function applyAccent(name: string | null | undefined) {
	const accent = accentOf(name);
	if (accent === "mint") {
		delete document.documentElement.dataset["accent"];
		localStorage.removeItem(ACCENT_KEY);
	} else {
		document.documentElement.dataset["accent"] = accent;
		localStorage.setItem(ACCENT_KEY, accent);
	}
}

/** Before the first render: the accent this window last drew. */
export function startAccent() {
	applyAccent(localStorage.getItem(ACCENT_KEY));
}
