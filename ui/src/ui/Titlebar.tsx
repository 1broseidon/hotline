import { useEffect, useState } from "react";
import { CloseIcon, MaximizeIcon, MinimizeIcon, RestoreIcon } from "../icons";
import { closeWindow, minimizeWindow, toggleMaximize, watchMaximized } from "../native";
import { windowTitle } from "../notify";
import { ToadMark } from "./ToadMark";

/**
 * The window's own top strip, drawn where the shell draws no frame (Linux,
 * Windows). It is the well itself: a drag region, the same title the task
 * bar shows, and the three controls flush to the corner where a hand
 * expects them, with the mark in the opposite corner where a frame keeps
 * its app icon. macOS keeps its traffic lights over the rail band and never
 * mounts this.
 *
 * On Linux the page is also the window's outline (index.css), so the
 * maximised state is stamped on the root for the corners to square off.
 */
export function Titlebar({ name }: { name: string | null }) {
	const [maximized, setMaximized] = useState(false);
	useEffect(
		() =>
			watchMaximized((value) => {
				setMaximized(value);
				document.documentElement.toggleAttribute("data-maximized", value);
			}),
		[],
	);
	return (
		<header className="titlebar">
			<div data-tauri-drag-region className="titlebar-drag" onDoubleClick={() => void toggleMaximize()} />
			<ToadMark className="titlebar-mark" width={18} />
			<p className="titlebar-title">{windowTitle(name)}</p>
			<div className="titlebar-controls">
				<button type="button" className="window-control" aria-label="Minimize" onClick={() => void minimizeWindow()}>
					<MinimizeIcon />
				</button>
				<button
					type="button"
					className="window-control"
					aria-label={maximized ? "Restore" : "Maximize"}
					onClick={() => void toggleMaximize()}
				>
					{maximized ? <RestoreIcon /> : <MaximizeIcon />}
				</button>
				<button type="button" className="window-control window-control-close" aria-label="Close" onClick={() => void closeWindow()}>
					<CloseIcon />
				</button>
			</div>
		</header>
	);
}
