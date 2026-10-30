import { chordKeys } from "../chords";
import { CloseIcon } from "../icons";

/**
 * A pane's close key: an X in its band's corner, on every platform. Where
 * the page draws the window's own controls they have a row of their own
 * (ui/Titlebar.tsx), so the two closes never stand side by side.
 */
export function PaneClose({ onClose }: { onClose(): void }) {
	return (
		<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
			<CloseIcon />
		</button>
	);
}
