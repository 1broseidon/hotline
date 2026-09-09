import { useEffect, useState } from "react";
import type { ConfigChoice } from "../generated/contract";
import { chordKeys } from "../chords";
import { SessionPickers } from "../components/Pickers";
import { CloseIcon, MaximizeIcon, MinimizeIcon, RestoreIcon, SearchIcon } from "../icons";
import { closeWindow, drawsFrame, minimizeWindow, toggleMaximize, watchWindowShape } from "../native";
import { windowTitle } from "../notify";
import type { RosterEntry } from "../wire";
import { ToadMark } from "./ToadMark";

/**
 * The window's top strip, on every platform: the well itself, with the
 * mark at the left corner where a frame keeps its app icon, the title in
 * the centre, and at the right the open teammate's model and effort and
 * the search. Where the shell draws no frame (Linux, Windows) the three
 * window controls sit flush to the right corner beyond them. macOS keeps
 * its traffic lights in the left corner and the strip leaves room for
 * them (index.css); the shell puts them on this strip's centre line. It
 * drags the window, and a double-click maximises.
 *
 * On Linux the page is also the window's outline, so the maximised state
 * is stamped on the root for the corners to square off.
 */
export function Titlebar({
	selected,
	models,
	searchable,
	searchOpen,
	onToggleSearch,
	onSaid,
}: {
	selected: RosterEntry | null;
	models: ConfigChoice[];
	/** A conversation is showing: the search has something to search. */
	searchable: boolean;
	searchOpen: boolean;
	onToggleSearch(): void;
	onSaid(said: string | null): void;
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
			<div data-tauri-drag-region className="titlebar-drag" onDoubleClick={() => void toggleMaximize()} />
			<ToadMark className="titlebar-mark" width={18} />
			<p className="titlebar-title">{windowTitle(selected?.persona.name ?? null)}</p>
			<div className="titlebar-tools">
				{selected !== null && <SessionPickers key={selected.persona.id} entry={selected} models={models} onSaid={onSaid} />}
				<button
					type="button"
					className="control btn-icon"
					title={`Search (${chordKeys("search")})`}
					aria-label="Search"
					aria-pressed={searchOpen}
					disabled={!searchable}
					onClick={onToggleSearch}
				>
					<SearchIcon />
				</button>
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
