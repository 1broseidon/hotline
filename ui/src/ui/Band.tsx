import type { ReactNode } from "react";
import { toggleMaximize } from "../native";

/**
 * The strip along the top of a pane: the window's chrome. It drags, a
 * double-click maximises, and the controls sit above the drag region so
 * they still take the click. `rail` is the band over the roster, which on
 * macOS leaves room for the traffic lights.
 */
export function Band({ children, rail = false }: { children: ReactNode; rail?: boolean }) {
	return (
		<div className={rail ? "band band-rail bg-sidebar" : "band bg-bg"}>
			<div data-tauri-drag-region className="band-drag" onDoubleClick={() => void toggleMaximize()} />
			<div className="band-row">{children}</div>
		</div>
	);
}
