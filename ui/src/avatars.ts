import { useMemo, useRef } from "react";
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

/** A colleague a line names: what to call them, and their picture's hash when they have one. */
export type Person = { name: string; hash?: string | undefined };

/** Every teammate by id, for lines that name a colleague rather than the teammate whose tape it is. */
export function peopleOf(roster: readonly { persona: { id: string; name: string; avatar?: { hash: string } | undefined } }[]): ReadonlyMap<string, Person> {
	return new Map(roster.map((entry) => [entry.persona.id, { name: entry.persona.name, hash: entry.persona.avatar?.hash }]));
}

/**
 * `peopleOf`, rebuilt only when a name or a picture changes. A roster event
 * hands over a new array for every state flicker; keyed on what the map holds,
 * the rows that read it are not asked to draw again.
 */
export function usePeople(roster: Parameters<typeof peopleOf>[0]): ReadonlyMap<string, Person> {
	const latest = useRef(roster);
	latest.current = roster;
	const key = roster.map((entry) => `${entry.persona.id}\u0000${entry.persona.name}\u0000${entry.persona.avatar?.hash ?? ""}`).join("\u0001");
	return useMemo(() => peopleOf(latest.current), [key]);
}

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
