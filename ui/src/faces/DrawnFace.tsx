import { useEffect, useState } from "react";
import { avatarUrl } from "../avatars";
import type { Clip } from "../generated/contract";
import { wire } from "../wire";
import { moodPicture } from "./drawing";
import type { Mood } from "./mood";
import rig from "./rig.css?raw";

const readDrawing = (personaId: string, hash: string, offset: number) => wire.command("look.clip_read", { personaId, hash, offset });

/** Each drawing's text by hash, and its picture per mood; both never change. */
const texts = new Map<string, Promise<string>>();
const pictures = new Map<string, string>();

function drawingText(personaId: string, hash: string): Promise<string> {
	let text = texts.get(hash);
	if (text === undefined) {
		text = avatarUrl(personaId, hash, readDrawing).then((url) => fetch(url).then((response) => response.text()));
		texts.set(hash, text);
		text.catch(() => texts.delete(hash));
	}
	return text;
}

async function pictureOf(personaId: string, hash: string, mood: Mood, still: boolean): Promise<string | null> {
	const key = `${hash}:${mood}:${still ? "still" : "moving"}`;
	const known = pictures.get(key);
	if (known !== undefined) return known;
	const svg = moodPicture(await drawingText(personaId, hash), mood, still, rig);
	if (svg === null) return null;
	const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
	pictures.set(key, url);
	return url;
}

/**
 * A face made of one drawing the person uploaded. Each mood is the drawing
 * with the rig's CSS inside it, shown as an image; the next mood's picture is
 * decoded before it replaces the one on screen, so a change never flashes.
 * A still face shows the mood's pose without moving.
 */
export function DrawnFace({ personaId, drawing, mood, still }: { personaId: string; drawing: Clip; mood: Mood; still: boolean }) {
	const [url, setUrl] = useState<string | null>(null);

	useEffect(() => {
		let gone = false;
		pictureOf(personaId, drawing.hash, mood, still).then(
			async (next) => {
				if (next === null) return;
				const image = new Image();
				image.src = next;
				await image.decode().catch(() => {});
				if (!gone) setUrl(next);
			},
			() => {},
		);
		return () => {
			gone = true;
		};
	}, [personaId, drawing.hash, mood, still]);

	if (url === null) return null;
	return <img className="face-drawn" src={url} alt="" draggable={false} />;
}
