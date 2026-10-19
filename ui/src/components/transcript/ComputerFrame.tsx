import { useEffect, useState } from "react";
import { Viewer } from "../../ui/Viewer";

/**
 * What the computer looked like. A thumbnail beside the words, because a
 * capture is evidence and not another message; pressed, it opens in the
 * viewer at a size that can be read.
 */
export function ComputerFrame({ eventId, dataUrl }: { eventId: string; dataUrl: string }) {
	const [open, setOpen] = useState(false);
	const url = frameUrl(eventId, dataUrl);
	useEffect(() => {
		hold(eventId);
		return () => release(eventId);
	}, [eventId]);
	return (
		<>
			<button
				type="button"
				className="picture-open mt-2 block w-28 overflow-hidden rounded-md border border-line bg-raised p-0 text-left"
				title="Open the capture"
				onClick={() => setOpen(true)}
			>
				<img src={url} alt="The computer's screen at capture" decoding="async" className="block w-full" />
			</button>
			{open && <Viewer src={url} alt="The computer's screen at capture" onClose={() => setOpen(false)} />}
		</>
	);
}

/** How many captures no row is showing stay as URLs, for a teammate switched back to. */
const KEPT = 24;
const frames = new Map<string, { url: string; shown: number }>();

/**
 * A capture as a blob URL, made once per event. A capture arrives as a data
 * URL of a whole screen, megabytes of text the page would otherwise parse
 * again each time a row with it is drawn; a frame never changes, so it is
 * decoded once and kept by its event's id. A URL a row is showing is never
 * let go; of the rest, the least recently drawn are, past `KEPT`, when a row
 * stops showing one. Not when one is made: a long tape makes all of its URLs
 * in one render, before any row has said it is showing its own.
 */
function frameUrl(eventId: string, dataUrl: string): string {
	const known = frames.get(eventId);
	if (known !== undefined) {
		frames.delete(eventId);
		frames.set(eventId, known);
		return known.url;
	}
	const blob = blobOf(dataUrl);
	if (blob === null) return dataUrl;
	const url = URL.createObjectURL(blob);
	frames.set(eventId, { url, shown: 0 });
	return url;
}

function hold(eventId: string): void {
	const frame = frames.get(eventId);
	if (frame !== undefined) frame.shown += 1;
}

function release(eventId: string): void {
	const frame = frames.get(eventId);
	if (frame !== undefined) frame.shown -= 1;
	letGo();
}

function letGo(): void {
	for (const [eventId, frame] of frames) {
		if (frames.size <= KEPT) return;
		if (frame.shown > 0) continue;
		URL.revokeObjectURL(frame.url);
		frames.delete(eventId);
	}
}

/** The bytes of a `data:` URL, or null for anything else, which is then drawn as it came. */
function blobOf(dataUrl: string): Blob | null {
	const comma = dataUrl.indexOf(",");
	if (!dataUrl.startsWith("data:") || comma < 0) return null;
	const header = dataUrl.slice("data:".length, comma);
	const type = header.split(";")[0] ?? "";
	const body = dataUrl.slice(comma + 1);
	try {
		if (!header.endsWith(";base64")) return new Blob([decodeURIComponent(body)], { type });
		const text = atob(body);
		const bytes = new Uint8Array(text.length);
		for (let index = 0; index < text.length; index++) bytes[index] = text.charCodeAt(index);
		return new Blob([bytes], { type });
	} catch {
		return null;
	}
}
