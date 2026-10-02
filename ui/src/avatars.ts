import type { FileChunk } from "./generated/contract";
import { wire } from "./wire";

export type AvatarRead = (personaId: string, hash: string, offset: number) => Promise<FileChunk>;

const readOverWire: AvatarRead = (personaId, hash, offset) => wire.command("avatar.read", { personaId, hash, offset });

/**
 * A teammate's picture as a URL the page can draw. A picture is named by the
 * hash of its bytes and never changes, so the URL is made once per hash and
 * kept; a failed read is forgotten, so the next ask tries again.
 */
const pictures = new Map<string, Promise<string>>();

export function avatarUrl(personaId: string, hash: string, read: AvatarRead = readOverWire): Promise<string> {
	const known = pictures.get(hash);
	if (known !== undefined) return known;
	const fetched = fetchAvatar(personaId, hash, read);
	pictures.set(hash, fetched);
	fetched.catch(() => {
		if (pictures.get(hash) === fetched) pictures.delete(hash);
	});
	return fetched;
}

async function fetchAvatar(personaId: string, hash: string, read: AvatarRead): Promise<string> {
	const parts: Uint8Array<ArrayBuffer>[] = [];
	let type = "image/png";
	let offset: number | undefined = 0;
	while (offset !== undefined) {
		const chunk: FileChunk = await read(personaId, hash, offset);
		type = chunk.mimeType;
		parts.push(bytesOf(chunk.data));
		offset = chunk.next;
	}
	return URL.createObjectURL(new Blob(parts, { type }));
}

function bytesOf(base64: string): Uint8Array<ArrayBuffer> {
	const text = atob(base64);
	const bytes = new Uint8Array(new ArrayBuffer(text.length));
	for (let index = 0; index < text.length; index++) bytes[index] = text.charCodeAt(index);
	return bytes;
}

/** Forgets every picture, for tests. */
export function forgetAvatars() {
	pictures.clear();
}
