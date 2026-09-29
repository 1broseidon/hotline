import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useEffect, useState } from "react";
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
 * The desktop's viewer while the container runs, or nothing. Asked every
 * few seconds while `wanted`, so a desktop that stopped on its own is
 * noticed without a reload; not asked at all otherwise.
 */
export function useComputerViewer(personaId: string, wanted: boolean): string | undefined {
	const [viewer, setViewer] = useState<string | undefined>(undefined);

	useEffect(() => {
		if (!wanted) {
			setViewer(undefined);
			return;
		}
		let gone = false;
		const ask = () => {
			void wire
				.command("computer.status", { personaId })
				.then((status) => {
					if (!gone) setViewer(status.state === "running" ? (status.viewer ?? bridgedViewer(personaId)) : undefined);
				})
				.catch(() => {
					if (!gone) setViewer(undefined);
				});
		};
		ask();
		const timer = setInterval(ask, COMPUTER_STATUS_EVERY_MS);
		return () => {
			gone = true;
			clearInterval(timer);
		};
	}, [personaId, wanted]);

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
