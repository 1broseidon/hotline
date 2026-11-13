import { type CSSProperties, useEffect, useState } from "react";
import { type AvatarRead, avatarUrl, type Picture } from "../avatars";

/**
 * A teammate's face: its picture in the circle when it has one (`hash` is the
 * picture's, from the teammate's record), else the first letter of the name on
 * a disc whose colour is hashed from the id, so a teammate keeps its colour
 * when the one above it is deleted. The disc stays behind a picture with
 * transparent parts and is left out behind one that fills its circle. The
 * initial also shows while the picture comes and if it cannot be read. The lightness and chroma are the tokens'
 * (ui/src/tokens.css); only the hue is chosen here. Red is missing on
 * purpose: it is the colour of something wrong.
 *
 * A face kept with its eyes shut (`shut`, the hash of its `shut` pose)
 * blinks while `blinking`: the shut picture is laid over the face and shown
 * for a moment every few seconds by a stepped CSS keyframe (index.css,
 * `face-blink`), which wakes no script. Its rhythm and phase come from the
 * id, so faces never blink in step. Not blinking, or with Reduce motion on,
 * the shut picture is not there at all.
 */
export function Avatar({
	id,
	name,
	size = 28,
	hash,
	shut,
	blinking = false,
	busy = false,
	read,
}: {
	id: string;
	name: string;
	size?: number;
	hash?: string | undefined;
	/** The hash of the same face with its eyes shut, from the record's `poses`. */
	shut?: string | undefined;
	/** The teammate is working: a face with `shut` blinks. */
	blinking?: boolean;
	/** Something is on its way: a session starting, a picture being drawn. A ring turns around the face. */
	busy?: boolean;
	/** A pinned desk, for pictures drawn after the window switches desks. */
	read?: AvatarRead | undefined;
}) {
	const picture = usePicture(id, hash, read);
	const wantsBlink = blinking && shut !== undefined && !reducedMotion();
	const shutPicture = usePicture(id, wantsBlink ? shut : undefined, read);
	const blinks = wantsBlink && picture !== undefined && shutPicture !== undefined;
	// The colour disc is for the initial and for a picture with transparent
	// parts. Behind a picture that fills the circle it would only bleed
	// through the circle's antialiased edge, as a thin ring (index.css).
	const face = (
		<span
			aria-hidden="true"
			className="avatar"
			data-fills={picture?.opaque === true || undefined}
			data-blinking={blinks || undefined}
			style={{
				width: size,
				height: size,
				fontSize: Math.round(size * 0.44),
				"--face": faceOf(id),
				...(blinks ? blinkRhythm(id) : {}),
			} as CSSProperties}
		>
			{picture === undefined ? initialOf(name) : <img className="avatar-picture" src={picture.url} alt="" draggable={false} />}
			{blinks && <img className="avatar-picture avatar-shut" src={shutPicture.url} alt="" draggable={false} />}
		</span>
	);
	if (!busy) return face;
	return (
		<span className="avatar-busy">
			{face}
			<span aria-hidden="true" className="avatar-ring" />
		</span>
	);
}

/** The picture once it has arrived; undefined before, and if it cannot be read. */
function usePicture(personaId: string, hash: string | undefined, read?: AvatarRead): Picture | undefined {
	const [loaded, setLoaded] = useState<{ hash: string; picture: Picture } | undefined>(undefined);
	useEffect(() => {
		if (hash === undefined) return;
		let gone = false;
		avatarUrl(personaId, hash, read).then(
			(picture) => {
				if (!gone) setLoaded({ hash, picture });
			},
			() => {},
		);
		return () => {
			gone = true;
		};
	}, [personaId, hash, read]);
	return hash !== undefined && loaded?.hash === hash ? loaded.picture : undefined;
}

/** The number a teammate's id hashes to, the same on the desk (`bubble` in face.rs). */
function idHash(personaId: string): number {
	let hash = 0;
	for (let index = 0; index < personaId.length; index++) {
		hash = (hash * 31 + personaId.charCodeAt(index)) % 1_000_003;
	}
	return hash;
}

/** Which of the seven face hues a teammate's id lands on, 0 to 6, from 70° in steps of 43°. */
export function faceHue(personaId: string): number {
	return idHash(personaId) % 7;
}

/**
 * How often a teammate's face blinks, 4.4 to 5.6 seconds, and how far into
 * that loop it starts (a negative delay), both from its id: two faces
 * working at once do not blink together.
 */
export function blinkRhythm(personaId: string): { "--blink-every": string; "--blink-phase": string } {
	const hash = idHash(personaId);
	const every = 4.4 + (hash % 25) * 0.05;
	const phase = (Math.floor(hash / 25) % 1000) / 1000;
	return { "--blink-every": `${every.toFixed(2)}s`, "--blink-phase": `${(-phase * every).toFixed(2)}s` };
}

/** Reduce motion is asked for: a face does not blink at all. */
function reducedMotion(): boolean {
	return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;
}

export function faceOf(personaId: string): string {
	return `oklch(var(--face-l) var(--face-c) ${70 + faceHue(personaId) * 43})`;
}

/** The first letter that is one, so "⌘kill bill" and " Ada" both read right. */
export function initialOf(name: string): string {
	return (name.match(/\p{L}|\p{N}/u)?.[0] ?? "?").toUpperCase();
}
