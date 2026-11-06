/**
 * A screen in the computer viewer's protocol, as the window's two viewers of
 * one draw and drive it: a teammate's computer on a server (computerViewer.ts)
 * and a teammate's desk browser on a server that streams it
 * (components/RemoteBrowser.tsx, task-390).
 *
 * Binary messages are frames: a 12-byte header of little-endian u16s (x, y,
 * width, height, screen width, screen height), then a PNG of that rectangle,
 * only for what changed. The person's input goes up as JSON (`move`,
 * `button`, `wheel`, `key`), none of it answered by a picture, so it never
 * waits behind one.
 */

type FrameHeader = { x: number; y: number; width: number; height: number; screenWidth: number; screenHeight: number };

/** Where a frame goes and how big its screen is, or null for a message too short to be one. */
function frameHeader(data: ArrayBuffer): FrameHeader | null {
	if (data.byteLength < 12) return null;
	const view = new DataView(data);
	const [x, y, width, height, screenWidth, screenHeight] = [0, 2, 4, 6, 8, 10].map((offset) => view.getUint16(offset, true)) as [
		number,
		number,
		number,
		number,
		number,
		number,
	];
	return { x, y, width, height, screenWidth, screenHeight };
}

/**
 * Paints frames onto a canvas in the order they came, the canvas sized to
 * the screen they say; `resized` is told when that size changes.
 */
export class ScreenPainter {
	private drawing: Promise<void> = Promise.resolve();

	constructor(
		private readonly canvas: HTMLCanvasElement,
		private readonly resized: () => void = () => {},
	) {}

	draw(data: ArrayBuffer): void {
		const header = frameHeader(data);
		if (header === null) return;
		if (this.canvas.width !== header.screenWidth || this.canvas.height !== header.screenHeight) {
			this.canvas.width = header.screenWidth;
			this.canvas.height = header.screenHeight;
			this.resized();
		}
		const blob = new Blob([data.slice(12)], { type: "image/png" });
		const context = this.canvas.getContext("2d");
		this.drawing = this.drawing
			.then(() => createImageBitmap(blob))
			.then((bitmap) => {
				context?.drawImage(bitmap, header.x, header.y);
				bitmap.close();
			})
			.catch(() => {});
	}
}

/** Where a pointer on the canvas is on the screen, in the screen's pixels, kept on it. */
function screenPoint(canvas: HTMLCanvasElement, event: { clientX: number; clientY: number }): { x: number; y: number } {
	const rect = canvas.getBoundingClientRect();
	const across = rect.width > 0 ? ((event.clientX - rect.left) * canvas.width) / rect.width : 0;
	const down = rect.height > 0 ? ((event.clientY - rect.top) * canvas.height) / rect.height : 0;
	return {
		x: Math.max(0, Math.min(canvas.width - 1, Math.round(across))),
		y: Math.max(0, Math.min(canvas.height - 1, Math.round(down))),
	};
}

/** Pixels of wheel that make one notch, from a trackpad or a mouse. */
const NOTCH = 50;
const BUTTONS: Record<number, number> = { 0: 1, 1: 2, 2: 3 };

export type Driving = {
	/** Sends one message up the screen's socket. */
	send(message: Record<string, unknown>): void;
	/** Whether the person drives the screen now; watching, nothing is sent. */
	driving(): boolean;
	/** A key this viewer keeps for itself (its own paste shortcut), handled there and never sent. */
	ownKey?(event: KeyboardEvent): boolean;
	/** The canvas lost the keyboard while driving. */
	blurred?(): void;
};

/**
 * The person's pointer, wheel and keys on `canvas`, sent as the protocol's
 * messages while they drive: one pointer position a frame, one notch per
 * fifty pixels of wheel, and each key down and up once (the screen repeats a
 * held key itself). Keys still down are let go when the window or the
 * canvas loses the keyboard, or a modifier left down would turn every later
 * key into a chord. Answers `letGo`, for a viewer that stops driving, and
 * `detach`.
 */
export function driveFrom(canvas: HTMLCanvasElement, driving: Driving): { letGo(): void; detach(): void } {
	const held = new Set<string>();
	const letGo = () => {
		for (const key of held) driving.send({ t: "key", key, down: false });
		held.clear();
	};
	let pending: { x: number; y: number } | null = null;
	const move = (event: PointerEvent) => {
		if (!driving.driving()) return;
		const first = pending === null;
		pending = screenPoint(canvas, event);
		if (first)
			requestAnimationFrame(() => {
				if (pending) driving.send({ t: "move", ...pending });
				pending = null;
			});
	};
	const down = (event: PointerEvent) => {
		if (!driving.driving()) return;
		canvas.focus();
		driving.send({ t: "move", ...screenPoint(canvas, event) });
		const button = BUTTONS[event.button];
		if (button !== undefined) driving.send({ t: "button", b: button, down: true });
	};
	const up = (event: PointerEvent) => {
		if (!driving.driving()) return;
		const button = BUTTONS[event.button];
		if (button !== undefined) driving.send({ t: "button", b: button, down: false });
	};
	const menu = (event: Event) => event.preventDefault();
	let wheelX = 0;
	let wheelY = 0;
	const wheel = (event: WheelEvent) => {
		if (!driving.driving()) return;
		event.preventDefault();
		const unit = event.deltaMode === 1 ? NOTCH : 1;
		wheelX += event.deltaX * unit;
		wheelY += event.deltaY * unit;
		while (Math.abs(wheelY) >= NOTCH) {
			driving.send({ t: "wheel", dy: Math.sign(wheelY) });
			wheelY -= Math.sign(wheelY) * NOTCH;
		}
		while (Math.abs(wheelX) >= NOTCH) {
			driving.send({ t: "wheel", dx: Math.sign(wheelX) });
			wheelX -= Math.sign(wheelX) * NOTCH;
		}
	};
	const keyDown = (event: KeyboardEvent) => {
		if (!driving.driving()) return;
		event.preventDefault();
		if (driving.ownKey?.(event)) return;
		if (!event.repeat) {
			held.add(event.key);
			driving.send({ t: "key", key: event.key, down: true });
		}
	};
	const keyUp = (event: KeyboardEvent) => {
		if (!driving.driving()) return;
		event.preventDefault();
		if (driving.ownKey?.(event)) return;
		held.delete(event.key);
		driving.send({ t: "key", key: event.key, down: false });
	};
	const blur = () => {
		letGo();
		if (driving.driving()) driving.blurred?.();
	};
	const hidden = () => {
		if (document.hidden) letGo();
	};
	canvas.addEventListener("pointermove", move);
	canvas.addEventListener("pointerdown", down);
	canvas.addEventListener("pointerup", up);
	canvas.addEventListener("contextmenu", menu);
	canvas.addEventListener("wheel", wheel, { passive: false });
	canvas.addEventListener("keydown", keyDown);
	canvas.addEventListener("keyup", keyUp);
	canvas.addEventListener("blur", blur);
	window.addEventListener("blur", letGo);
	document.addEventListener("visibilitychange", hidden);
	return {
		letGo,
		detach() {
			canvas.removeEventListener("pointermove", move);
			canvas.removeEventListener("pointerdown", down);
			canvas.removeEventListener("pointerup", up);
			canvas.removeEventListener("contextmenu", menu);
			canvas.removeEventListener("wheel", wheel);
			canvas.removeEventListener("keydown", keyDown);
			canvas.removeEventListener("keyup", keyUp);
			canvas.removeEventListener("blur", blur);
			window.removeEventListener("blur", letGo);
			document.removeEventListener("visibilitychange", hidden);
		},
	};
}
