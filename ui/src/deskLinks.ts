import { useSyncExternalStore } from "react";
import type { DeskLink } from "./generated/contract";
import { allDesks, LOCAL_DESK, wireFor } from "./desks";

/**
 * The desks this computer's desk is linked with (ADR 0003), so their
 * teammates can see and message each other. Links are this computer's
 * desk's: it makes them, over the owner seat this window already holds on
 * the other desk, and it dials them. A link's channel opens and closes with
 * no event the window hears, so what is shown is asked for again every few
 * seconds while anything is showing it, and not at all otherwise.
 */

let links: DeskLink[] = [];
const listeners = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | null = null;
const EVERY_MS = 4000;

/**
 * Tells the listeners only when the links changed. The answer is a new list
 * every time it is asked for, and an idle window asks every few seconds: a
 * render for each would wake the page for nothing.
 */
function publish(next: DeskLink[]) {
	if (JSON.stringify(next) === JSON.stringify(links)) return;
	links = next;
	for (const listener of listeners) listener();
}

function local() {
	return allDesks().some((desk) => desk.id === LOCAL_DESK) ? wireFor(LOCAL_DESK) : null;
}

/** Asks this computer's desk again. A refusal leaves what it last said. */
async function readDeskLinks(): Promise<void> {
	const wire = local();
	if (wire === null) {
		publish([]);
		return;
	}
	try {
		publish(await wire.command("desks.links", {}));
	} catch {
		// Reconnecting, or a core from before links: nothing new to say.
	}
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	if (timer === null) {
		void readDeskLinks();
		timer = setInterval(() => void readDeskLinks(), EVERY_MS);
	}
	return () => {
		listeners.delete(listener);
		if (listeners.size === 0 && timer !== null) {
			clearInterval(timer);
			timer = null;
		}
	};
}

const unwatched = () => () => {};

/**
 * The links, asked for again every few seconds while `watching`. A caller
 * that shows them only some of the time (a menu while it is open) watches
 * only then, and reads what was last said otherwise.
 */
export function useDeskLinks(watching = true): DeskLink[] {
	return useSyncExternalStore(watching ? subscribe : unwatched, () => links);
}

/** Links this computer's desk with a desk this window is connected to. */
export async function linkDesk(deskId: string): Promise<void> {
	const wire = local();
	if (wire === null) throw new Error("Only a window on this computer's desk can link it with another.");
	await wire.command("desks.link", { deskId });
	await readDeskLinks();
}

/** Ends a link, here and on the other desk. */
export async function unlinkDesk(deskId: string): Promise<void> {
	const wire = local();
	if (wire === null) throw new Error("Only a window on this computer's desk can end its links.");
	await wire.command("desks.unlink", { deskId });
	await readDeskLinks();
}

/** How a link is doing, in a few words for a menu or a row. */
export function linkWords(link: DeskLink): string {
	if (link.state === "connected") return "Teammates linked";
	return link.error ? `Linked, can't reach it: ${link.error}` : "Linked, reconnecting…";
}
