import type { ReactNode } from "react";
import { toggleMaximize } from "../native";

/**
 * The strip that is the window's chrome: it drags, and a double-click
 * maximises. Buttons and fields sit on top of the drag region so they still
 * receive the click. The traffic-light inset is CSS, on the rail only.
 */
export function Chrome({
	children,
	className = "",
}: {
	children: ReactNode;
	className?: string;
}) {
	return (
		<div className={`chrome ${className}`}>
			<div
				data-tauri-drag-region
				className="chrome-drag"
				onDoubleClick={() => void toggleMaximize()}
			/>
			<div className="chrome-row">{children}</div>
		</div>
	);
}
