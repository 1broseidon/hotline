import type { ReactNode } from "react";
import { chordKeys } from "../chords";
import { ArrowLeftIcon } from "../icons";
import { toggleMaximize } from "../native";

/**
 * The strip along the top of a pane: the window's chrome. It drags, a
 * double-click maximises, and the controls sit above the drag region so
 * they still take the click. `rail` is the band over the roster, which on
 * macOS leaves room for the traffic lights.
 */
export function Band({ children, rail = false }: { children: ReactNode; rail?: boolean }) {
	return (
		<div className={rail ? "band band-rail" : "band"}>
			<div data-tauri-drag-region className="band-drag" onDoubleClick={() => void toggleMaximize()} />
			<div className="band-row">{children}</div>
		</div>
	);
}

/** The step back to the rail, at the head of a band in a narrow window. */
export function BackKey({ onBack }: { onBack(): void }) {
	return (
		<button
			type="button"
			className="control btn-icon -ml-1"
			title={`Back (${chordKeys("close")})`}
			aria-label="Back to the team"
			onClick={onBack}
		>
			<ArrowLeftIcon />
		</button>
	);
}
