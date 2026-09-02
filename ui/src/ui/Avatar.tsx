/**
 * A teammate's face: the first letter of the name on a disc whose colour is
 * hashed from the id, so a teammate keeps its colour when the one above it
 * is deleted. The lightness and chroma are the tokens' (ui/src/tokens.css);
 * only the hue is chosen here. Red is missing on purpose: it is the colour of
 * something wrong.
 */
export function Avatar({ id, name, size = 28 }: { id: string; name: string; size?: number }) {
	return (
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
			{initialOf(name)}
		</span>
	);
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
