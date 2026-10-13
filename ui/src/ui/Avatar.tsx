import { useEffect, useState } from "react";
import { type AvatarRead, avatarUrl } from "../avatars";
import { type Moving, moves, useFace } from "../faces/face";
import { MovingFace } from "../faces/MovingFace";
import { useHeldMood, useMood } from "../faces/mood";

/**
 * A teammate's face: its picture in the circle when it has one (`hash` is the
 * picture's, from the teammate's record), else the first letter of the name on
 * a disc whose colour is hashed from the id, so a teammate keeps its colour
 * when the one above it is deleted. The initial also shows while the picture
 * comes and if it cannot be read. The lightness and chroma are the tokens'
 * (ui/src/tokens.css); only the hue is chosen here. Red is missing on
 * purpose: it is the colour of something wrong. A teammate whose look is an
 * video face (faces/) shows that instead, in the mood its state calls for,
 * when this window draws faces.
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
	const look = useFace(id);
	const moving = moves(look) ? look : undefined;
	const url = usePicture(id, look === null ? hash : undefined, read);
	const face = moving !== undefined ? (
		<span aria-hidden="true" className="avatar avatar-face" style={{ width: size, height: size }}>
			<MoodFace id={id} face={moving} still={size < MOVES_FROM} />
		</span>
	) : (
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

/**
 * The smallest face that moves. A teammate's face shows in the rail and the
 * header as well as above the conversation; only the big one moves, and the
 * small ones hold its mood's pose, so one face is animating, not three.
 */
const MOVES_FROM = 64;

function MoodFace({ id, face, still }: { id: string; face: Moving; still: boolean }) {
	return <MovingFace personaId={id} face={face} mood={useHeldMood(useMood(id))} still={still} />;
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

export function faceOf(personaId: string): string {
	let hash = 0;
	for (let index = 0; index < personaId.length; index++) {
		hash = (hash * 31 + personaId.charCodeAt(index)) % 1_000_003;
	}
	return `oklch(var(--face-l) var(--face-c) ${70 + (hash % 7) * 43})`;
}

/** The first letter that is one, so "⌘kill bill" and " Ada" both read right. */
export function initialOf(name: string): string {
	return (name.match(/\p{L}|\p{N}/u)?.[0] ?? "?").toUpperCase();
}
