import { type ReactNode, useEffect, useState } from "react";
import { chordKeys } from "../chords";
import { CloseIcon, MaximizeIcon, MinimizeIcon, PersonPlusIcon, RestoreIcon, SidebarIcon } from "../icons";
import { closeWindow, drawsFrame, minimizeWindow, toggleMaximize, watchWindowShape } from "../native";
import { HotlineMark } from "./HotlineMark";

/**
 * The window's top strip, on every platform: the well itself, with the
 * rail's show-and-hide key at the left corner, the mark alone in the
 * centre, and a new teammate at the right, with Settings and the search
 * beside it while the rail is closed. It is the window's, not a
 * teammate's: who is open, and their model and effort, are in the pane's
 * band. Where the shell draws no frame (Linux, Windows) the three
 * window controls sit flush to the right corner beyond them. macOS keeps
 * its traffic lights in the left corner and the strip leaves room for
 * them (index.css); the shell puts them on this strip's centre line. It
 * drags the window, and a double-click maximises.
 *
 * On Linux the page is also the window's outline, so the maximised state
 * is stamped on the root for the corners to square off.
 */
export function Titlebar({
	rail,
	search,
	deskKey,
	onNew,
	settings,
	bare = false,
}: {
	/** Whether the rail is open, and the key that opens and closes it; none in a narrow window. */
	rail?: { open: boolean; onToggle(): void } | undefined;
	/**
	 * The team's search and the desk's call key. The rail's dock holds them;
	 * while the rail is closed they stand here instead, at the head of the tools.
	 */
	search?: ReactNode;
	deskKey?: ReactNode;
	/** A new teammate, at the right. */
	onNew?: (() => void) | undefined;
	/** Settings: the rail's dock holds it, at the window's bottom-left corner; while the rail is closed it stands here, first of the tools. */
	settings?: ReactNode;
	/** The welcome: the mark and the window's own controls, nothing that acts on a room that is not there yet. */
	bare?: boolean;
}) {
	const frame = drawsFrame();
	const [maximized, setMaximized] = useState(false);
	useEffect(() => {
		if (!frame) return;
		return watchWindowShape(({ maximized: value }) => {
			setMaximized(value);
			document.documentElement.toggleAttribute("data-maximized", value);
		});
	}, [frame]);
	return (
		<header className="titlebar">
			<div data-tauri-drag-region aria-hidden="true" className="titlebar-drag" onDoubleClick={() => void toggleMaximize()} />
			<div className="titlebar-lead">
				{rail !== undefined && !bare && (
					<button
						type="button"
						className="control btn-icon"
						title={`${rail.open ? "Hide" : "Show"} the team (${chordKeys("sidebar")})`}
						aria-label={rail.open ? "Hide the team" : "Show the team"}
						onClick={rail.onToggle}
					>
						<SidebarIcon />
					</button>
				)}
			</div>
			<p className="titlebar-title">
				{/* The mark alone: the app's name. Whose conversation this is lives
				    once, in the pane's band; the task bar, which has no band,
				    names them (windowTitle). */}
				<HotlineMark className="titlebar-mark" width={18} plain label="Hotline" />
			</p>
			<div className="titlebar-tools" hidden={bare}>
				{/* Settings, search and the desk are the rail's; with the rail closed they stand here. */}
				{(rail === undefined || !rail.open) && settings}
				{rail !== undefined && !rail.open && (
					<>
						{search}
						{deskKey}
					</>
				)}
				{onNew !== undefined && (
					<button type="button" className="control btn-icon" title={`New teammate (${chordKeys("new-teammate")})`} aria-label="New teammate" onClick={onNew}>
						<PersonPlusIcon />
					</button>
				)}
			</div>
			{frame && (
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
			)}
		</header>
	);
}
