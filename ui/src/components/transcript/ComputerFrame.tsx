import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { platform } from "../../native";
import { atRest, stir } from "../../rest";
import { Viewer } from "../../ui/Viewer";

/**
 * What the computer looked like. A thumbnail beside the words, because a
 * capture is evidence and not another message; pressed, it opens in the
 * viewer at a size that can be read.
 *
 * A capture is a whole screen, some 1,500 px across, and its thumbnail is
 * 110. Drawn from the capture itself, the engine decodes all of it, four
 * bytes a pixel (5 MB for one), and holds that while the row is in the page
 * and for some time after. So on macOS the thumbnail is drawn from a copy
 * of the capture made at the thumbnail's own pixels (`small`, below):
 * pixel for pixel what WebKit draws there in the thumbnail of a capture it
 * has decoded whole, as it has once the capture was opened in the viewer.
 * (One never opened it drew from a picture its decoder had scaled, a shade
 * crisper; every thumbnail now looks as the opened ones did.) The capture
 * itself is then drawn only until that copy is made, in the viewer, and in
 * the thumbnail from the moment it is pressed until something else is: so
 * the viewer opens on a picture already decoded, and a drag or the
 * system's Copy Image, which take what the thumbnail holds, take the
 * whole capture.
 *
 * Elsewhere the capture is drawn as it always was. WebKitGTK scales a
 * picture it draws small far better than any scaling it offers a page
 * (measured, 2.52: every copy a page can make differs from the engine's
 * own thumbnail by 9 of 255 on average, and looks jagged beside it), so a
 * copy there would be a worse thumbnail; WebView2 was not measured.
 */
export function ComputerFrame({ eventId, dataUrl }: { eventId: string; dataUrl: string }) {
	const [open, setOpen] = useState(false);
	const [pressed, setPressed] = useState(false);
	const button = useRef<HTMLButtonElement>(null);
	const small = useSmall(eventId, dataUrl);
	const wanted = open || pressed || small === undefined;
	const whole = wanted ? frameUrl(eventId, dataUrl) : undefined;
	// Held in the commit that draws it: a copy landing between this render
	// and a later effect would let the capture go from under it.
	useLayoutEffect(() => {
		if (!wanted) return;
		hold(eventId);
		return () => release(eventId);
	}, [eventId, wanted]);
	// Pressed lasts until the next press that is not on it: through the
	// click that opens it, a drag out of the window, the system's menu.
	useEffect(() => {
		if (!pressed) return;
		const elsewhere = (event: PointerEvent) => {
			if (!button.current?.contains(event.target as Node | null)) setPressed(false);
		};
		document.addEventListener("pointerdown", elsewhere, true);
		return () => document.removeEventListener("pointerdown", elsewhere, true);
	}, [pressed]);
	return (
		<>
			<button
				ref={button}
				type="button"
				className="picture-open mt-2 block w-28 overflow-hidden rounded-md border border-line bg-raised p-0 text-left"
				title="Open the capture"
				onPointerDown={() => setPressed(true)}
				onClick={() => setOpen(true)}
			>
				{/* One element whichever picture it holds, so the engine draws the one it had until the next has loaded.
				    With the copy it is given the capture's own shape: the copy is a whole number of pixels tall, which the
				    capture's box (110 by 61.875 for a screen of 1568 by 882) is not, and a row is as tall as its box. */}
				<img
					src={small === undefined || pressed ? whole : small.url}
					style={small !== undefined ? { aspectRatio: `${small.width} / ${small.height}` } : undefined}
					alt="The computer's screen at capture"
					decoding={small === undefined ? "async" : "sync"}
					className="block w-full"
				/>
			</button>
			{open && whole !== undefined && <Viewer src={whole} alt="The computer's screen at capture" fact="Screen capture" onClose={() => setOpen(false)} />}
		</>
	);
}

/** How wide a thumbnail's picture is drawn, in CSS px: the button's `w-28` less its border on either side. */
const DRAWN = 110;

/**
 * A capture's small copy: a PNG as wide as the thumbnail is on this screen
 * (`pixels`), with the capture's own `width` and `height`, and an image
 * already loaded from it, which keeps it where a row coming back into the
 * page draws it in its first frame instead of loading it again.
 */
type Small = { url: string; image: HTMLImageElement; pixels: number; width: number; height: number };

/** How many small copies are kept, the least recently drawn let go first. One is some tens of kilobytes. */
const SMALL_KEPT = 256;
const smalls = new Map<string, Small>();
/** Captures no small copy can be made of here: they are drawn whole, as every capture once was. */
const unmade = new Set<string>();
/** Captures waiting for their copy, in the order rows first held them: the first is the one being made. */
const line: { eventId: string; dataUrl: string; pixels: number }[] = [];
let making = false;

/* Counts the copies made: what a thumbnail reads `smalls` again for. */
let made = 0;
const watching = new Set<() => void>();
function onSmalls(listener: () => void): () => void {
	watching.add(listener);
	return () => watching.delete(listener);
}

/**
 * The capture's small copy at this screen's pixels, once there is one. A
 * row that holds a capture asks for it, the rows the transcript measures
 * off to the side included, and it is made whether or not the row is still
 * there: a row far up a thread is measured long before it is scrolled to,
 * and by then its thumbnail has never needed the capture.
 */
function useSmall(eventId: string, dataUrl: string): Small | undefined {
	useSyncExternalStore(onSmalls, () => made);
	const pixels = Math.round(DRAWN * useSyncExternalStore(onPixelRatio, pixelRatio));
	const known = smalls.get(eventId);
	const small = known?.pixels === pixels ? known : undefined;
	useEffect(() => {
		if (small === undefined) ask(eventId, dataUrl, pixels);
	}, [eventId, dataUrl, pixels, small]);
	if (small !== undefined) {
		// Drawn, it is the newest: the last to be let go.
		smalls.delete(eventId);
		smalls.set(eventId, small);
	}
	return small;
}

const pixelRatio = () => window.devicePixelRatio || 1;

/** Calls `changed` when the window moves to a screen with another pixel ratio. A query matches one ratio, so it is asked again each time. */
function onPixelRatio(changed: () => void): () => void {
	let query: MediaQueryList | undefined;
	const moved = () => {
		listen();
		changed();
	};
	const listen = () => {
		query?.removeEventListener("change", moved);
		query = window.matchMedia?.(`(resolution: ${pixelRatio()}dppx)`);
		query?.addEventListener?.("change", moved);
	};
	listen();
	return () => query?.removeEventListener?.("change", moved);
}

function ask(eventId: string, dataUrl: string, pixels: number): void {
	// Only where the copy is the picture the engine itself would draw (see the top of this file).
	if (platform() !== "macos" || typeof createImageBitmap !== "function" || unmade.has(eventId)) return;
	const waiting = line.find((one) => one.eventId === eventId);
	if (waiting !== undefined) waiting.pixels = pixels;
	else line.push({ eventId, dataUrl, pixels });
	work();
}

/**
 * Makes the next copy once the window is at rest (rest.ts): decoding a
 * whole screen to make one is some 10 to 30 ms. One to a turn, and the end
 * of a turn counts as a stir, so the window draws frames and takes input
 * between two.
 */
function work(): void {
	if (making || line.length === 0) return;
	making = true;
	atRest(() => {
		const next = line[0]!;
		const { pixels } = next;
		const done = (small: Small | null) => {
			// Asked for at another size meanwhile (the window changed screens): it stays in the line for that one.
			if (next.pixels === pixels) line.shift();
			if (small === null) unmade.add(next.eventId);
			else keep(next.eventId, small);
			making = false;
			stir();
			work();
		};
		shrink(next.dataUrl, pixels).then(done, () => done(null));
	});
}

function keep(eventId: string, small: Small): void {
	const was = smalls.get(eventId);
	if (was !== undefined) URL.revokeObjectURL(was.url);
	smalls.delete(eventId);
	smalls.set(eventId, small);
	for (const [oldest, one] of smalls) {
		if (smalls.size <= SMALL_KEPT) break;
		URL.revokeObjectURL(one.url);
		smalls.delete(oldest);
	}
	// The capture itself is no longer what its thumbnail is drawn from.
	letGo();
	made++;
	for (const listener of watching) listener();
}

/**
 * A capture made `pixels` wide, by the engine's best filter: the picture the
 * engine itself draws in the thumbnail once it has decoded the whole
 * capture, as it has for any capture opened in the viewer. Scaled as it is
 * decoded, and decoded twice, once to learn its size: WebKit scales an
 * `ImageBitmap` drawn to a canvas with no filter whatever is asked for,
 * and an image element decoded for this keeps its megabytes long after.
 * Null for a capture no wider than its thumbnail, which is drawn as it is.
 */
async function shrink(dataUrl: string, pixels: number): Promise<Small | null> {
	const blob = blobOf(dataUrl);
	if (blob === null) return null;
	const whole = await createImageBitmap(blob);
	const { width, height } = whole;
	whole.close();
	if (width <= pixels) return null;
	const copy = await createImageBitmap(blob, { resizeWidth: pixels, resizeHeight: Math.round((pixels * height) / width), resizeQuality: "high" });
	const canvas = document.createElement("canvas");
	canvas.width = copy.width;
	canvas.height = copy.height;
	canvas.getContext("2d")?.drawImage(copy, 0, 0);
	copy.close();
	const png = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
	if (png === null) return null;
	const url = URL.createObjectURL(png);
	const image = new Image();
	image.src = url;
	try {
		await image.decode();
	} catch {
		URL.revokeObjectURL(url);
		return null;
	}
	return { url, image, pixels, width, height };
}

/** How many captures no row is showing stay as URLs where no small copy can be made, for a teammate switched back to. */
const KEPT = 24;
const frames = new Map<string, { url: string; shown: number }>();

/**
 * A capture as a blob URL, made once per event. A capture arrives as a data
 * URL of a whole screen, megabytes of text the page would otherwise parse
 * again each time a row with it is drawn; a frame never changes, so it is
 * decoded once and kept by its event's id. A URL a row is showing is never
 * let go. Of the rest, one whose capture has its small copy goes at once,
 * and of the others the least recently drawn go, past `KEPT`, when a row
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
		if (frame.shown > 0) continue;
		if (frames.size <= KEPT && !smalls.has(eventId)) continue;
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
