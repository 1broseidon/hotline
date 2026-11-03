import { faceHue } from "./ui/Avatar";

/**
 * The colours a teammate's bubbles may take (decision-43): grey, which every
 * teammate answers in until the person picks another, and the faces' hues
 * but gold, whose tint would sit beside the amber of a card waiting on you.
 * The names are the core's (`BUBBLES` in the contract); tokens.css draws each
 * as `--bubble-<name>` in both palettes.
 */
export type Bubble = "grey" | "lime" | "green" | "teal" | "blue" | "violet" | "pink";

/** By face hue index (ui/Avatar.tsx faceHue): gold, the first, has no bubble. */
const BY_FACE: readonly (Bubble | null)[] = [null, "lime", "green", "teal", "blue", "violet", "pink"];

const NAMES: Record<Bubble, string> = {
	grey: "Grey",
	lime: "Lime",
	green: "Green",
	teal: "Teal",
	blue: "Blue",
	violet: "Violet",
	pink: "Pink",
};

/** A persona's saved bubble, or grey for none and for a name this window doesn't know. */
export function bubbleOf(saved: string | undefined): Bubble {
	return saved !== undefined && saved in NAMES ? (saved as Bubble) : "grey";
}

/** Grey, then the teammate's own face hue when it has a bubble, then the rest in hue order. */
export function bubbleChoices(personaId: string): { id: Bubble; name: string }[] {
	const own = BY_FACE[faceHue(personaId)] ?? null;
	const order: Bubble[] = ["grey", ...(own !== null ? [own] : []), ...BY_FACE.filter((one): one is Bubble => one !== null && one !== own)];
	return order.map((id) => ({ id, name: NAMES[id] }));
}
