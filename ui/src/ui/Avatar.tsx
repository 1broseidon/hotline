import { useEffect, useState } from "react";
import { type AvatarRead, avatarUrl } from "../avatars";

/**
 * A teammate's face: its picture in the circle when it has one (`hash` is the
 * picture's, from the teammate's record), else the first letter of the name on
 * a disc whose colour is hashed from the id, so a teammate keeps its colour
 * when the one above it is deleted. The initial also shows while the picture
 * comes and if it cannot be read. The lightness and chroma are the tokens'
 * (ui/src/tokens.css); only the hue is chosen here. Red is missing on
 * purpose: it is the colour of something wrong.
 */
export function Avatar({
	id,
	name,
	size = 28,
	hash,
	busy = false,
	read,
}: {
	id: string;
	name: string;
	size?: number;
	hash?: string | undefined;
	/** Something is on its way: a session starting, a picture being drawn. A ring turns around the face. */
	busy?: boolean;
	/** A pinned desk, for pictures drawn after the window switches desks. */
	read?: AvatarRead | undefined;
}) {
	const url = usePicture(id, hash, read);
	const face = (
		<span
			aria-hidden="true"
			className="avatar"
			style={{
				width: size,
				height: size,
				fontSize: Math.round(size * 0.44),
				background: faceOf(id),
			}}
		>
			{url === undefined ? initialOf(name) : <img className="avatar-picture" src={url} alt="" draggable={false} />}
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

/** The picture's URL once it has arrived; undefined before, and if it cannot be read. */
function usePicture(personaId: string, hash: string | undefined, read?: AvatarRead): string | undefined {
	const [loaded, setLoaded] = useState<{ hash: string; url: string } | undefined>(undefined);
	useEffect(() => {
		if (hash === undefined) return;
		let gone = false;
		avatarUrl(personaId, hash, read).then(
			(url) => {
				if (!gone) setLoaded({ hash, url });
			},
			() => {},
		);
		return () => {
			gone = true;
		};
	}, [personaId, hash, read]);
	return hash !== undefined && loaded?.hash === hash ? loaded.url : undefined;
}

function faceOf(personaId: string): string {
	let hash = 0;
	for (let index = 0; index < personaId.length; index++) {
		hash = (hash * 31 + personaId.charCodeAt(index)) % 1_000_003;
	}
	return `oklch(var(--face-l) var(--face-c) ${70 + (hash % 7) * 43})`;
}

/** The first letter that is one, so "⌘kill bill" and " Ada" both read right. */
function initialOf(name: string): string {
	return (name.match(/\p{L}|\p{N}/u)?.[0] ?? "?").toUpperCase();
}
