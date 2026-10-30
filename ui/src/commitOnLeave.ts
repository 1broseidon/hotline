import { useEffect, useRef } from "react";

/**
 * A field that saves when it loses focus loses its words when it leaves the
 * screen instead: React removes an input without a blur, so closing Settings
 * or the teammate pane with the cursor still in the field never saved it.
 * This runs the field's own commit then, and when the window is closed. The
 * commit compares against what is stored, as each already does on blur, so
 * running it after a blur already saved changes nothing.
 */
export function useCommitOnLeave(commit: () => void) {
	const latest = useRef(commit);
	latest.current = commit;
	useEffect(() => {
		const leave = () => latest.current();
		window.addEventListener("pagehide", leave);
		return () => {
			window.removeEventListener("pagehide", leave);
			leave();
		};
	}, []);
}
