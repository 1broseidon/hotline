/**
 * Tells the room when the person is at this window, so their phone stays
 * quiet about what the window already shows (push.rs in the core).
 *
 * At the window means focused, showing, and used lately: a window left in
 * focus on a desk nobody sits at is not being looked at. The room takes one
 * "yes" as good for two minutes, so while the person keeps using the window
 * it is said again every thirty seconds at most. A blur or a hidden window
 * says "no" at once.
 */

import { wire } from "./wire";

const AGAIN_MS = 30_000;
const USE = ["pointerdown", "pointermove", "keydown", "wheel"] as const;

let saidAt = 0;
let saidLooking = false;

function say(looking: boolean): void {
	saidAt = Date.now();
	saidLooking = looking;
	// A socket that is not up yet says nothing; the next touch says it again.
	wire.command("desk.looking", { looking }).catch(() => {
		saidLooking = false;
	});
}

function used(): void {
	if (!document.hasFocus() || document.visibilityState !== "visible") return;
	if (saidLooking && Date.now() - saidAt < AGAIN_MS) return;
	say(true);
}

function left(): void {
	if (saidLooking) say(false);
}

function shown(): void {
	if (document.visibilityState === "visible") used();
	else left();
}

/** Starts watching; the returned function stops. */
export function watchLooking(): () => void {
	for (const kind of USE) window.addEventListener(kind, used, { capture: true, passive: true });
	window.addEventListener("focus", used);
	window.addEventListener("blur", left);
	document.addEventListener("visibilitychange", shown);
	return () => {
		for (const kind of USE) window.removeEventListener(kind, used, { capture: true });
		window.removeEventListener("focus", used);
		window.removeEventListener("blur", left);
		document.removeEventListener("visibilitychange", shown);
	};
}
