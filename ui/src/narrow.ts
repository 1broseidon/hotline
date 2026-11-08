import { useEffect, useState } from "react";

/**
 * Whether the window is too narrow for the rail and a pane side by side.
 * Below this the window shows one at a time, the way a phone does; the
 * inspector has its own, wider cut-off in index.css, because it needs the
 * conversation beside it to make sense and the rail does not.
 */
const NARROW = "(max-width: 719px)";

/* Asked of the window as each use mounts, not once when the module loads,
 * so the query is always the window's own: a module read before the window
 * it runs in (a test's DOM installed later) would keep a stale one. */
export function useNarrow(): boolean {
	const [narrow, setNarrow] = useState(() => window.matchMedia(NARROW).matches);
	useEffect(() => {
		const query = window.matchMedia(NARROW);
		const update = () => setNarrow(query.matches);
		update();
		query.addEventListener("change", update);
		return () => query.removeEventListener("change", update);
	}, []);
	return narrow;
}
