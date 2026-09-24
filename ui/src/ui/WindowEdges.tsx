/**
 * The edges of a frameless window on Linux, where it is taken to resize.
 *
 * The shell draws no frame there, so it draws no resize cursor either:
 * tauri starts a resize from a press within 5px (times the GTK scale) of
 * the window's edge, but the page under the pointer keeps its own cursor,
 * and the edge is found by guessing. These are that same band, wearing the
 * cursors a frame would (index.css); the press itself stays tauri's, so a
 * strip that did anything with it would resize twice. Windows and macOS
 * draw their own frame's cursors.
 */
const EDGES = ["n", "ne", "e", "se", "s", "sw", "w", "nw"] as const;

export function WindowEdges() {
	return (
		<div aria-hidden="true">
			{EDGES.map((edge) => (
				// A press here starts a resize, not a text selection.
				<div key={edge} className={`window-edge window-edge-${edge}`} onMouseDown={(event) => event.preventDefault()} />
			))}
		</div>
	);
}
