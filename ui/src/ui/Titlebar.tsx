import { type ReactNode, useEffect, useRef, useState } from "react";
import { chordKeys } from "../chords";
import { CloseIcon, MaximizeIcon, MinimizeIcon, PersonPlusIcon, RestoreIcon, SidebarIcon } from "../icons";
import { closeWindow, drawsFrame, minimizeWindow, platform, toggleMaximize, watchWindowShape } from "../native";
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

/** Where macOS's traffic lights end, with a gap after them (lights.rs seats them 12 in). */
const LIGHTS_RIGHT = 76;

/**
 * The room's chrome. On macOS the panes run to the window's top, with no
 * strip of their own: the rail's key stands over the rail's head, past the
 * traffic lights, with the search beside it while the rail is closed, and
 * whatever pane's band runs under it is measured and makes room for it
 * (`--lead-room`), so the faces-only rail and a closed rail keep their titles
 * clear. A call's card that would stand under it drops below the top bar
 * instead (`--chrome-drop`), keeping the pane's whole width. Where the shell
 * draws no frame (Linux, Windows) the window's controls have a row of their
 * own, with the rail's key and the app's name, and the panes start under it:
 * beside the window's close, a pane's own close read as a second one.
 */
export function FlushChrome({
	rail,
	width,
	search,
}: {
	/** `key: false` shows no key to hide or show it (Settings' sections stay out). */
	rail: { open: boolean; onToggle(): void; key?: boolean };
	/** The rail's width while it is open: its head is the lead's drag area. */
	width: number;
	search: ReactNode;
}) {
	const frame = drawsFrame();
	const [maximized, setMaximized] = useState(false);
	const lead = useRef<HTMLDivElement>(null);
	useEffect(() => {
		if (!frame) return;
		return watchWindowShape(({ maximized: value }) => {
			setMaximized(value);
			document.documentElement.toggleAttribute("data-maximized", value);
		});
	}, [frame]);
	useEffect(() => {
		// Under a row of the window's own nothing stands under the chrome.
		if (frame) return;
		// At most one fit a frame, and never put off: while the rail slides the
		// bands move every frame, and a fit that waited for them to stop would
		// leave a title under the lead until the slide ended.
		let pending = 0;
		const fit = () => {
			if (pending !== 0) return;
			pending = window.setTimeout(() => {
				pending = 0;
				const keys = [...(lead.current?.children ?? [])].filter((child) => !child.classList.contains("chrome-lead-drag"));
				// macOS's traffic lights are the lead's too, though they are the window's to draw.
				const lights = platform() === "macos" && !document.documentElement.hasAttribute("data-fullscreen") ? LIGHTS_RIGHT : 0;
				const leadRight = Math.max(lights, ...keys.map((key) => key.getBoundingClientRect().right));
				// macOS draws its window's controls in the lead's corner, not here.
				const controlsLeft = Number.POSITIVE_INFINITY;
				for (const band of document.querySelectorAll<HTMLElement>("#root .band")) {
					sized.observe(band);
					const box = band.getBoundingClientRect();
					const top = box.top < 40;
					band.style.setProperty("--lead-room", top && box.left < leadRight ? `${Math.ceil(leadRight - box.left)}px` : "0px");
					band.style.setProperty("--controls-room", top && box.right > controlsLeft ? `${Math.ceil(box.right - controlsLeft)}px` : "0px");
				}
				// A call's card moves when it drops, so it is measured by the pane it
				// stands at the top of, which does not.
				for (const card of document.querySelectorAll<HTMLElement>("#root .call-head")) {
					if (card.parentElement === null) continue;
					sized.observe(card.parentElement);
					const pane = card.parentElement.getBoundingClientRect();
					const under = pane.top < 40 && (pane.left < leadRight || pane.right > controlsLeft);
					card.style.setProperty("--chrome-drop", under ? "40px" : "0px");
				}
			}, 16);
		};
		fit();
		const root = document.getElementById("root") ?? document.body;
		const added = new window.MutationObserver(fit);
		added.observe(root, { childList: true, subtree: true });
		const sized = new window.ResizeObserver(fit);
		sized.observe(root);
		if (lead.current !== null) sized.observe(lead.current);
		window.addEventListener("resize", fit);
		return () => {
			window.clearTimeout(pending);
			added.disconnect();
			sized.disconnect();
			window.removeEventListener("resize", fit);
		};
	}, [frame]);
	const leadKeys = (
		<>
			{rail.key !== false && <button
				type="button"
				className="control btn-icon"
				title={`${rail.open ? "Hide" : "Show"} the team (${chordKeys("sidebar")})`}
				aria-label={rail.open ? "Hide the team" : "Show the team"}
				onClick={rail.onToggle}
			>
				<SidebarIcon />
			</button>}
			{!rail.open && search}
		</>
	);
	if (frame) {
		// Windows and Linux: the window's controls have a row of their own
		// across the window's top, with the rail's key and the app's name, and
		// the panes start under it. A pane's close is its own X again, not one
		// beside the window's (task-308 QA).
		return (
			<header className="chrome-strip">
				<div data-tauri-drag-region aria-hidden="true" className="chrome-strip-drag" onDoubleClick={() => void toggleMaximize()} />
				<div className="chrome-strip-lead">{leadKeys}</div>
				<p className="chrome-strip-title">
					<HotlineMark className="titlebar-mark" width={16} plain label="Hotline" />
				</p>
				<div className="chrome-strip-controls">
					<button type="button" className="window-control" aria-label="Minimize" onClick={() => void minimizeWindow()}>
						<MinimizeIcon />
					</button>
					<button type="button" className="window-control" aria-label={maximized ? "Restore" : "Maximize"} onClick={() => void toggleMaximize()}>
						{maximized ? <RestoreIcon /> : <MaximizeIcon />}
					</button>
					<button type="button" className="window-control window-control-close" aria-label="Close" onClick={() => void closeWindow()}>
						<CloseIcon />
					</button>
				</div>
			</header>
		);
	}
	return (
		<div ref={lead} className="chrome-lead" style={rail.open ? { width } : undefined}>
			{rail.open && <div data-tauri-drag-region aria-hidden="true" className="chrome-lead-drag" onDoubleClick={() => void toggleMaximize()} />}
			{leadKeys}
		</div>
	);
}
