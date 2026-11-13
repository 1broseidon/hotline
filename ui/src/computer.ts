import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useEffect, useRef, useState } from "react";
import { activeDeskId, allDesks } from "./desks";
import { isDesktop, openLink } from "./native";
import { wire } from "./wire";

/**
 * A teammate's desktop, seen from the window. The container serves a
 * viewer on loopback while it runs; Hotline shows it in a window of its own
 * rather than the person's browser, so the screen sits beside the room
 * and a second press finds the window already open instead of opening
 * another. Asking after the desktop never wakes it — a pane that peeks
 * is not a pane that starts containers.
 *
 * On a desk on a server the container is not reachable from here, and the
 * desk keeps its viewer address to itself. The window opens its own viewer
 * page instead (computerViewer.ts), which reaches the screen through the
 * desk's bridge.
 */

/** How often a pane that shows the desktop asks whether it is still there. */
export const COMPUTER_STATUS_EVERY_MS = 5000;

/** Said on the window when a teammate's computer was started or stopped from here. */
const MOVED = "hotline:computer-moved";

/** Tells everything showing this teammate's computer to look again now, rather than at its next tick. */
export function computerMoved(personaId: string): void {
	window.dispatchEvent(new window.CustomEvent(MOVED, { detail: personaId }));
}

/** Open the teammate's desktop, or bring the window already showing it forward. */
export async function openComputer(personaId: string, name: string, viewer: string): Promise<void> {
	if (!isDesktop()) {
		await openLink(viewer);
		return;
	}
	const label = `computer-${personaId}`;
	try {
		const shown = await WebviewWindow.getByLabel(label);
		if (shown !== null) {
			await shown.setFocus();
			return;
		}
		// The window's own viewer page learns the name from its address too.
		// It takes dropped files in its own Files panel, so the page, not the
		// shell, is who hears a drop.
		const bundled = viewer.startsWith("computer.html#");
		const url = bundled ? `${viewer}&${new URLSearchParams({ name })}` : viewer;
		new WebviewWindow(label, { url, title: `${name}'s computer`, width: 1280, height: 800, dragDropEnabled: !bundled });
	} catch {
		await openLink(viewer);
	}
}

/**
 * How often the conversation's computer key looks again while its window is
 * in front: the person is there to press it.
 */
export const COMPUTER_KEY_FOCUSED_MS = 10_000;
/**
 * How often it looks again while the window is on screen behind another
 * app's. An idle window that asks every few seconds wakes its page for
 * nothing, and WebKit keeps memory from every such wake.
 */
export const COMPUTER_KEY_BEHIND_MS = 60_000;

/**
 * The desktop's viewer while the container runs, or nothing. The desk says
 * nothing on the wire when a computer starts or stops, so it is asked: at
 * once, then every COMPUTER_KEY_FOCUSED_MS while the window is in front and
 * every COMPUTER_KEY_BEHIND_MS while it is on screen behind another app, not
 * at all while it cannot be seen. It is asked again at once when the window
 * comes in front or into view, when the socket opens, when the pane starts
 * or stops the computer (`computerMoved`), and whenever `nudge` changes
 * (the conversation passes its turn's state, so a computer the teammate
 * started in a turn is seen when the turn ends). An answer that says what
 * the last one said draws nothing.
 */
export function useComputerViewer(personaId: string, wanted: boolean, nudge?: string): string | undefined {
	const [viewer, setViewer] = useState<string | undefined>(undefined);
	// What the key last showed, read outside React: an answer equal to it
	// sets no state at all, since even an equal one can cost React a render.
	const shown = useRef<string | undefined>(undefined);
	const look = useRef<() => void>(() => {});

	useEffect(() => {
		if (!wanted) {
			shown.current = undefined;
			setViewer(undefined);
			look.current = () => {};
			return;
		}
		let gone = false;
		let timer: ReturnType<typeof setTimeout> | null = null;
		const stop = () => {
			if (timer !== null) clearTimeout(timer);
			timer = null;
		};
		const next = () => {
			stop();
			if (gone || document.visibilityState === "hidden") return;
			timer = setTimeout(ask, document.hasFocus() ? COMPUTER_KEY_FOCUSED_MS : COMPUTER_KEY_BEHIND_MS);
		};
		const show = (seen: string | undefined) => {
			if (gone || shown.current === seen) return;
			shown.current = seen;
			setViewer(seen);
		};
		const ask = () => {
			stop();
			void wire
				.command("computer.status", { personaId })
				.then(
					(status) => show(status.state === "running" ? (status.viewer ?? bridgedViewer(personaId)) : undefined),
					() => show(undefined),
				)
				.finally(next);
		};
		look.current = ask;
		ask();
		// A Stop or Start from the pane is seen at once: the band's key goes
		// with the computer, not up to a tick later.
		const moved = (event: Event) => {
			if ((event as CustomEvent<string>).detail === personaId) ask();
		};
		const seen = () => (document.visibilityState === "hidden" ? stop() : ask());
		let first = true;
		const unwatch = wire.onConnection((state) => {
			if (first) first = false;
			else if (state === "open") ask();
		});
		window.addEventListener(MOVED, moved);
		window.addEventListener("focus", ask);
		document.addEventListener("visibilitychange", seen);
		return () => {
			gone = true;
			stop();
			look.current = () => {};
			unwatch();
			window.removeEventListener(MOVED, moved);
			window.removeEventListener("focus", ask);
			document.removeEventListener("visibilitychange", seen);
		};
	}, [personaId, wanted]);

	// The first nudge is the one the key opened with, and it has just asked.
	const nudged = useRef(nudge);
	useEffect(() => {
		if (nudged.current === nudge) return;
		nudged.current = nudge;
		look.current();
	}, [nudge]);

	return viewer;
}

/**
 * The window's own viewer for a running computer on a desk on a server,
 * through its bridge; undefined on this computer's desk, whose status
 * names the container's viewer.
 */
function bridgedViewer(personaId: string): string | undefined {
	const id = activeDeskId();
	const desk = allDesks().find((one) => one.id === id);
	if (desk?.kind !== "remote") return undefined;
	// Never the desk's owner token: the viewer asks the shell for one that
	// opens this teammate's screen once (BRO-148).
	const params = new URLSearchParams({ desk: desk.id, persona: personaId });
	return `computer.html#${params.toString()}`;
}
