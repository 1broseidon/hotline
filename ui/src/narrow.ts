import { useEffect, useState } from "react";

/**
 * Whether the window is too narrow for the rail and a pane side by side.
 * Below this the window shows one at a time, the way a phone does; the
 * inspector has its own, wider cut-off in index.css, because it needs the
 * conversation beside it to make sense and the rail does not.
 */
const NARROW = window.matchMedia("(max-width: 719px)");

export function useNarrow(): boolean {
	const [narrow, setNarrow] = useState(NARROW.matches);
	useEffect(() => {
		const update = () => setNarrow(NARROW.matches);
		NARROW.addEventListener("change", update);
		return () => NARROW.removeEventListener("change", update);
	}, []);
	return narrow;
}
