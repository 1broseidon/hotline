import { useMemo, useRef } from "react";
import { type Bubble, bubbleOf } from "./bubbles";
import type { FileChunk } from "./generated/contract";
import { wire } from "./wire";

export type AvatarRead = (personaId: string, hash: string, offset: number) => Promise<FileChunk>;

const readOverWire: AvatarRead = (personaId, hash, offset) => wire.command("avatar.read", { personaId, hash, offset });

/**
 * A teammate's picture as a URL the page can draw, and whether it fills its
 * circle. A picture is named by the hash of its bytes and never changes, so
 * the URL is made once per hash and kept; a failed read is forgotten, so the
 * next ask tries again.
 */
export type Picture = { url: string; opaque: boolean };

const pictures = new Map<string, Promise<Picture>>();

/** A colleague a line names: what to call them, their picture's hash and their bubble's colour unless it is grey. */
export type Person = { name: string; hash?: string | undefined; bubble?: string | undefined };

/** Every teammate by id, for lines that name a colleague rather than the teammate whose tape it is. */
export function peopleOf(roster: readonly { persona: { id: string; name: string; avatar?: { hash: string } | undefined; bubble?: string | undefined } }[]): ReadonlyMap<string, Person> {
	return new Map(roster.map((entry) => [entry.persona.id, { name: entry.persona.name, hash: entry.persona.avatar?.hash, bubble: tinted(bubbleOf(entry.persona.bubble, entry.persona.id)) }]));
}

/** A grey bubble is a plain one, drawn without a colour of its own. */
function tinted(bubble: Bubble): Bubble | undefined {
	return bubble === "grey" ? undefined : bubble;
}

/**
 * `peopleOf`, rebuilt only when a name, a picture or a bubble changes. A roster event
 * hands over a new array for every state flicker; keyed on what the map holds,
 * the rows that read it are not asked to draw again.
 */
export function usePeople(roster: Parameters<typeof peopleOf>[0]): ReadonlyMap<string, Person> {
	const latest = useRef(roster);
	latest.current = roster;
	const key = roster.map((entry) => `${entry.persona.id}\u0000${entry.persona.name}\u0000${entry.persona.avatar?.hash ?? ""}\u0000${entry.persona.bubble ?? ""}`).join("\u0001");
	// biome-ignore lint/correctness/useExhaustiveDependencies: the key is the roster's names, pictures and bubbles, which peopleOf reads through latest.
	return useMemo(() => peopleOf(latest.current), [key]);
}

export function avatarUrl(personaId: string, hash: string, read: AvatarRead = readOverWire): Promise<Picture> {
	const known = pictures.get(hash);
	if (known !== undefined) return known;
	const fetched = fetchAvatar(personaId, hash, read);
	pictures.set(hash, fetched);
	fetched.catch(() => {
		if (pictures.get(hash) === fetched) pictures.delete(hash);
	});
	return fetched;
}

async function fetchAvatar(personaId: string, hash: string, read: AvatarRead): Promise<Picture> {
	const parts: Uint8Array<ArrayBuffer>[] = [];
	let type = "image/png";
	let offset: number | undefined = 0;
	while (offset !== undefined) {
		const chunk: FileChunk = await read(personaId, hash, offset);
		type = chunk.mimeType;
		parts.push(bytesOf(chunk.data));
		offset = chunk.next;
	}
	return { url: URL.createObjectURL(new Blob(parts, { type })), opaque: opaquePng(parts[0]) };
}

/**
 * Whether a PNG has no transparency at all: no alpha channel (its colour
 * type is grey or RGB) and no transparency chunk. The desk keeps a picture
 * with no transparent pixel this way, so a face on a ground of its own is
 * drawn with nothing behind it.
 */
export function opaquePng(bytes: Uint8Array | undefined): boolean {
	if (bytes === undefined || bytes.length < 33) return false;
	const signature = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
	if (signature.some((byte, index) => bytes[index] !== byte)) return false;
	const colourType = bytes[25];
	if (colourType !== 0 && colourType !== 2) return false;
	// Chunks run from byte 8: a length, a type, the data and a check. A
	// transparency chunk comes before the image data, if there is one.
	const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
	let at = 8;
	while (at + 8 <= bytes.length) {
		const kind = String.fromCharCode(bytes[at + 4] ?? 0, bytes[at + 5] ?? 0, bytes[at + 6] ?? 0, bytes[at + 7] ?? 0);
		if (kind === "tRNS") return false;
		if (kind === "IDAT") return true;
		at += 12 + view.getUint32(at);
	}
	return false;
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
