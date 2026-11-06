import type { BrowserInput, BrowserSnapshot, Viewport } from "./generated/contract";

/**
 * What the window's viewer of a desk browser on another desk decides
 * (components/RemoteBrowser.tsx): where a click on the picture lands on the
 * page, which keys go to the page and as what, and how input waits its turn.
 * The phone drives the same commands the same way (hotline-mobile's
 * browser-drive.ts). The lock is the pane's bar's too (browserBar.ts), so
 * this module reaches nothing but the contract's types.
 */

/** How often a person driving the browser asks for a fresh picture; the desk pictures at most every 250 ms. */
export const POLL_MS = 400;
/** The most text one `browser.input` types, in characters, as the desk takes it. */
const TEXT_LIMIT = 10_000;
/** The farthest one scroll goes either way, in CSS pixels, as the desk takes it. */
const SCROLL_LIMIT = 20_000;

/**
 * The glyph before an address: a house for this machine's own and the
 * private ranges, the open lock for a certificate the person chose to visit
 * anyway, a lock for https, and none for the rest.
 */
export function lockKind(url: string, excepted = false): "local" | "excepted" | "secure" | "" {
	try {
		const parsed = new URL(url);
		if (parsed.hostname === "localhost" || /^(127\.|10\.|192\.168\.|\[::1\])/.test(parsed.hostname)) return "local";
		if (excepted) return "excepted";
		return parsed.protocol === "https:" ? "secure" : "";
	} catch {
		return "";
	}
}

/**
 * Where a point on the screen falls on the page, in its CSS pixels: the
 * picture is drawn whole and centred in `box`, scaled to fit, so the point
 * is first taken off any margin beside it, then scaled to the page's `size`.
 * Null off the picture.
 */
export function pagePoint(
	box: { left: number; top: number; width: number; height: number },
	size: Viewport,
	clientX: number,
	clientY: number,
): { x: number; y: number } | null {
	if (size.width <= 0 || size.height <= 0 || box.width <= 0 || box.height <= 0) return null;
	const scale = Math.min(box.width / size.width, box.height / size.height);
	const x = (clientX - box.left - (box.width - size.width * scale) / 2) / scale;
	const y = (clientY - box.top - (box.height - size.height * scale) / 2) / scale;
	if (!(x >= 0 && y >= 0 && x <= size.width && y <= size.height)) return null;
	return { x: Math.round(x * 10) / 10, y: Math.round(y * 10) / 10 };
}

/** The keys the desk presses by name; anything else that is not one character stays here. */
const NAMED_KEYS = new Set([
	"Enter",
	"Tab",
	"Escape",
	"Backspace",
	"Delete",
	"ArrowUp",
	"ArrowDown",
	"ArrowLeft",
	"ArrowRight",
	"Home",
	"End",
	"PageUp",
	"PageDown",
]);

/**
 * What a key pressed on the picture is for the page: a named key, or one
 * character of text (a space included, so typed words arrive whole). A
 * shortcut held with Command or Control is the window's, as Paste is, and
 * goes nowhere.
 */
export function keyInput(press: { key: string; shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }): BrowserInput | null {
	if (press.ctrlKey || press.metaKey) return null;
	if (press.key === "Tab" && press.shiftKey) return { kind: "key", key: "Shift+Tab" };
	if (NAMED_KEYS.has(press.key)) return { kind: "key", key: press.key };
	if ([...press.key].length === 1) return { kind: "text", text: press.key };
	return null;
}

/** A scroll by CSS pixels, at a point on the page when there is one, within what the desk takes. */
export function scrollInput(dx: number, dy: number, at: { x: number; y: number } | null): BrowserInput {
	const clamp = (value: number) => Math.round(Math.max(-SCROLL_LIMIT, Math.min(SCROLL_LIMIT, value)));
	const input: BrowserInput = { kind: "scroll", dx: clamp(dx), dy: clamp(dy) };
	return at === null ? input : { ...input, x: at.x, y: at.y };
}

/** Text as the desk takes it: in pieces of at most its limit. */
export function textInputs(text: string): BrowserInput[] {
	const chars = [...text];
	const pieces: BrowserInput[] = [];
	for (let at = 0; at < chars.length; at += TEXT_LIMIT) pieces.push({ kind: "text", text: chars.slice(at, at + TEXT_LIMIT).join("") });
	return pieces;
}

/**
 * Two waiting inputs as one, where they add up: text joins to the desk's
 * limit, and scrolls add to its limit at the later point, since a wheel
 * moved while it turned meant where it ended. Null where they do not.
 */
export function join(a: BrowserInput, b: BrowserInput): BrowserInput | null {
	if (a.kind === "text" && b.kind === "text") {
		const text = a.text + b.text;
		return [...text].length <= TEXT_LIMIT ? { kind: "text", text } : null;
	}
	if (a.kind === "scroll" && b.kind === "scroll") {
		const dx = a.dx + b.dx;
		const dy = a.dy + b.dy;
		if (Math.abs(dx) > SCROLL_LIMIT || Math.abs(dy) > SCROLL_LIMIT) return null;
		return { ...b, dx, dy };
	}
	return null;
}

/** Whether a picture should replace the one shown: never an older one, however late it answers. */
export function isNewer(shown: BrowserSnapshot | undefined, next: BrowserSnapshot | null | undefined): next is BrowserSnapshot {
	if (!next?.jpeg) return false;
	return shown === undefined || next.at >= shown.at;
}

/**
 * The person's input, one `browser.input` at a time and in order. While one
 * is on its way the next ones wait, and waiting ones that add up are joined,
 * so a fast typist or a spinning wheel sends a few commands rather than a
 * flood, and Enter still lands after the words before it. A refusal drops
 * what waited behind it, which would only be refused the same way.
 */
export class InputQueue {
	private waiting: BrowserInput[] = [];
	private busy = false;

	constructor(
		private readonly deliver: (input: BrowserInput) => Promise<BrowserSnapshot | null>,
		private readonly onPicture: (snapshot: BrowserSnapshot | null) => void,
		private readonly onRefused: (error: unknown) => void,
	) {}

	push(input: BrowserInput): void {
		const last = this.waiting.at(-1);
		const joined = last === undefined ? null : join(last, input);
		if (joined === null) this.waiting.push(input);
		else this.waiting[this.waiting.length - 1] = joined;
		void this.pump();
	}

	/** Nothing waiting goes: the browser was handed back, or the viewer closed. */
	clear(): void {
		this.waiting = [];
	}

	private async pump(): Promise<void> {
		if (this.busy) return;
		this.busy = true;
		try {
			for (let next = this.waiting.shift(); next !== undefined; next = this.waiting.shift()) {
				try {
					this.onPicture(await this.deliver(next));
				} catch (error) {
					this.waiting = [];
					this.onRefused(error);
				}
			}
		} finally {
			this.busy = false;
		}
	}
}
