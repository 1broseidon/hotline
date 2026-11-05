import { createContext, type ReactNode, useContext, useEffect, useState } from "react";
import { CheckIcon, CopyIcon, ExpandIcon } from "../icons";
import { writeClipboard } from "../native";

/**
 * An object a message carries, as the full-window view draws it: its kind,
 * for the view's title, and its markdown, which the view renders with the
 * same components the bubble did.
 */
export type FullObject = { kind: "table" | "code" | "diagram"; text: string; title?: string };

/**
 * How an object in a message opens over the whole window. The window provides
 * it around the conversation; inside the full view itself, and in a document
 * already open at full size, there is no provider, and so no Expand key.
 */
export const OpenFull = createContext<((object: FullObject) => void) | null>(null);

/**
 * The one sheet every object in a message sits on: the full width of its
 * bubble, edge to edge, no border and no card, with a quiet band under it.
 * A head names it, when naming it says something the object does not, and
 * its keys wait for a hover. A table needs no name: its keys float over its
 * head row.
 */
export function Sheet({ kind, label, keys, children }: { kind: "table" | "code" | "diagram"; label?: ReactNode; keys: ReactNode; children: ReactNode }) {
	return (
		<div className="sheet" data-sheet={kind} data-bare={label === undefined || undefined}>
			<div className="sheet-head">
				{label !== undefined && <span className="sheet-label">{label}</span>}
				<span className="sheet-keys">{keys}</span>
			</div>
			{children}
		</div>
	);
}

/** A quiet key on a sheet's head: a glyph, its word kept for a screen reader and the tooltip; `pressed` when it is a toggle. */
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
			<span className="sheet-key-word">{label}</span>
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

/** Expand, where there is a full view to open the object in. */
export function ExpandKey({ object }: { object: FullObject }) {
	const open = useContext(OpenFull);
	if (open === null) return null;
	return <SheetKey icon={<ExpandIcon />} label="Expand" title="Full screen" onClick={() => open(object)} />;
}
