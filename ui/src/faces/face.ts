import type { Look } from "../generated/contract";
import { useLook } from "./mood";
import { type FacePrefs, useFacePrefs } from "./prefs";

/**
 * What a teammate's face draws as, from its look and this window's
 * preference. `null` is the face as it always was: the picture, else the
 * initial. A video face needs its idle clip, which every mood without a clip
 * of its own plays; without one the teammate keeps its picture.
 */
export type Face = { kind: "initial" } | { kind: "video"; clips: Look["clips"] };

export function faceFor(prefs: FacePrefs, look: Look | undefined): Face | null {
	if (!prefs.on || look === undefined) return null;
	switch (look.style) {
		case "initial":
			return { kind: "initial" };
		case "photo":
			return null;
		case "video":
			return look.clips.idle !== undefined ? { kind: "video", clips: look.clips } : null;
	}
}

export function useFace(personaId: string): Face | null {
	return faceFor(useFacePrefs(), useLook(personaId));
}

/** Whether the face moves, so the big face and the activity mark have something to show. */
export function moves(face: Face | null): face is Extract<Face, { kind: "video" }> {
	return face?.kind === "video";
}
