import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useEffect, useState } from "react";
import { isDesktop, openLink } from "./native";
import { wire } from "./wire";

/**
 * A teammate's desktop, seen from the window. The container serves a
 * viewer on loopback while it runs; Hotline shows it in a window of its own
 * rather than the person's browser, so the screen sits beside the room
 * and a second press finds the window already open instead of opening
 * another. Asking after the desktop never wakes it — a pane that peeks
 * is not a pane that starts containers.
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
		new WebviewWindow(label, { url: viewer, title: `${name}'s computer`, width: 1280, height: 800 });
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
					if (!gone) setViewer(status.state === "running" ? status.viewer : undefined);
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
