import { createContext, type ReactNode, useContext, useEffect, useState } from "react";
import { CheckIcon, CopyIcon, ExpandIcon } from "../icons";
import { writeClipboard } from "../native";

/**
 * An object a message carries, as the pane draws it at full width: its kind,
 * for the pane's title, and its markdown, which the pane renders with the
 * same components the bubble did.
 */
export type PaneObject = { kind: "table" | "diagram"; text: string };

/**
 * How an object in a message opens in the pane beside the conversation. The
 * window provides it around the conversation and the pane; where there is no
 * pane to open into (Settings' release notes, the pane's own copy of the
 * object) there is no provider, and so no Open key.
 */
export const OpenInPane = createContext<((object: PaneObject) => void) | null>(null);

/**
 * The one sheet every object in a message sits in (docs/design/inline-premium.html):
 * sunk into the bubble, with a head that names it in the instrument face at
 * the left and its quiet keys at the right.
 */
export function Sheet({ kind, label, keys, children }: { kind: "table" | "code" | "diagram"; label: ReactNode; keys: ReactNode; children: ReactNode }) {
	return (
		<div className="sheet" data-sheet={kind}>
			<div className="sheet-head">
				<span className="sheet-label">{label}</span>
				<span className="sheet-keys">{keys}</span>
			</div>
			{children}
		</div>
	);
}

/** A quiet key on a sheet's head: a glyph and a word; `pressed` when it is a toggle. */
export function SheetKey({
	icon,
	label,
	title,
	pressed,
	onClick,
}: {
	icon?: ReactNode;
	label: string;
	title: string;
	pressed?: boolean;
	onClick(): void;
}) {
	return (
		<button type="button" className="sheet-key" title={title} aria-pressed={pressed} onClick={onClick}>
			{icon}
			{label}
		</button>
	);
}

/** Copy, which says Copied for a moment once the clipboard has it. */
export function CopyKey({ text, title }: { text: string; title: string }) {
	const [copied, setCopied] = useState(false);
	useEffect(() => {
		if (!copied) return;
		const timer = window.setTimeout(() => setCopied(false), 1500);
		return () => window.clearTimeout(timer);
	}, [copied]);
	return (
		<SheetKey
			icon={copied ? <CheckIcon /> : <CopyIcon />}
			label={copied ? "Copied" : "Copy"}
			title={title}
			onClick={() => void writeClipboard(text).then(() => setCopied(true))}
		/>
	);
}

/** Open, where there is a pane to open the object in. */
export function OpenKey({ object }: { object: PaneObject }) {
	const open = useContext(OpenInPane);
	if (open === null) return null;
	return <SheetKey icon={<ExpandIcon />} label="Open" title="Open in the pane" onClick={() => open(object)} />;
}
