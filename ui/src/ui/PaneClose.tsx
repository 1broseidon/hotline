import { chordKeys } from "../chords";
import { CloseIcon } from "../icons";
import { drawsFrame } from "../native";

/**
 * A pane's close key. On macOS it is an X, the window's own controls being
 * the traffic lights at the other corner. Where the page draws the window's
 * controls (Windows, Linux) a pane's X stood beside the window's own close,
 * two X's in one corner, so there it says Close in words.
 */
export function PaneClose({ onClose }: { onClose(): void }) {
	const title = `Close (${chordKeys("close")})`;
	if (drawsFrame()) {
		return (
			<button type="button" className="control btn btn-sm" title={title} onClick={onClose}>
				Close
			</button>
		);
	}
	return (
		<button type="button" className="control btn-icon" title={title} aria-label="Close" onClick={onClose}>
			<CloseIcon />
		</button>
	);
}
