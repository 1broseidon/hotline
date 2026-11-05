import { createContext, type FocusEvent, type KeyboardEvent, type ReactNode, useContext, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { CheckIcon, CopyIcon, ExpandIcon } from "../icons";
import { writeClipboard } from "../native";

import { type Said, SaidBy } from "../ui/OpenBand";

export { SaidBy };

/**
 * An object a message carries, as the open view draws it: its kind; its
 * markdown, which the view renders with the same components the bubble did
 * (a diagram's is its mermaid source); the object as a fact for the band
 * ("Table · 7 rows × 8 columns"); the sentence that introduced it, if one
 * did; and, once opened from a bubble, who said it and the sheet it grows
 * from and goes back to.
 */
export type FullObject = {
	kind: "table" | "code" | "diagram";
	text: string;
	fact: string;
	caption?: string | undefined;
	from?: Said | undefined;
	sheet?: HTMLElement | undefined;
};

/**
 * How an object in a message opens in the conversation's place. The window
 * provides it around the conversation; inside the open view itself, and in
 * a document already open at full size, there is no provider, and nothing
 * there is cut.
 */
export const OpenFull = createContext<((object: FullObject) => void) | null>(null);

/**
 * Opening an object from the sheet it sits on: the window's OpenFull, with
 * the bubble's teammate and time and the sheet itself added. Null where
 * nothing opens.
 */
export function useOpen(): ((object: FullObject, sheet: HTMLElement | null) => void) | null {
	const openFull = useContext(OpenFull);
	const said = useContext(SaidBy);
	if (openFull === null) return null;
	return (object, sheet) => openFull({ ...object, ...(said !== null ? { from: said } : {}), ...(sheet !== null ? { sheet } : {}) });
}

/**
 * A bubble's one toolbar, as a sheet in it sees it: where to put its keys
 * while it is pointed at (`node`, the toolbar's own slot after the
 * bubble's react, reply and copy), and `offer`, which moves the toolbar
 * level with the sheet and makes its one Copy take the sheet's content.
 * The bubble puts both back once the pointer is on its words again or has
 * left it, not as it leaves the sheet, so the keys stay to be reached. Outside a
 * bubble there is none, and a sheet keeps its keys on its own head.
 */
export type BlockCopy = { text: string; title: string };
export type BubbleTools = {
	node: HTMLElement | null;
	active: string | null;
	offer(id: string, top: number, copy?: BlockCopy): void;
	/** The keyboard has left the sheet for somewhere other than the toolbar: the toolbar is the message's again. */
	release(id: string): void;
};
export const BubbleToolbar = createContext<BubbleTools | null>(null);

/**
 * How a sheet is too much for its bubble: wider than it, longer than a
 * glance, or both, and what it is, said under the cut. A cut sheet shows
 * what fits, faded at the edge it goes on past, and opens whole on a click,
 * as the phone's card does; nothing in a bubble scrolls sideways.
 */
export type Cut = { summary: string; wide: boolean; tall: boolean };

/**
 * The one sheet every object in a message sits on: the full width of its
 * bubble, edge to edge, no border and no card, with a quiet band under it.
 * A head names it, when naming it says something the object does not, and
 * its keys wait for a hover. A table needs no name: its keys float over its
 * head row.
 */
export function Sheet({
	kind,
	label,
	keys,
	copy,
	open,
	cut,
	children,
}: {
	kind: "table" | "code" | "diagram";
	label?: ReactNode;
	keys: ReactNode;
	/** What Copy takes from this sheet: the bubble's one Copy while pointed at, else a key of its own. */
	copy?: BlockCopy;
	/** What a click opens full screen once the sheet is cut. */
	open?: FullObject;
	/** Set when the object is too much for the bubble; ignored where there is no full view. */
	cut?: Cut | null;
	children: ReactNode;
}) {
	const tools = useContext(BubbleToolbar);
	const openFrom = useOpen();
	const self = useRef<HTMLDivElement>(null);
	const opens = cut != null && open !== undefined && openFrom !== null ? () => openFrom(open, self.current) : null;
	const id = useId();
	// In a bubble the keys join the bubble's toolbar, one belt, while this
	// sheet is the one pointed at; elsewhere they stand on its own head.
	// What Copy takes is read when it is pressed, not when the pointer came:
	// a block still streaming in is copied as it stands then.
	const latest = useRef(copy);
	latest.current = copy;
	const live: BlockCopy | undefined =
		copy === undefined
			? undefined
			: {
					get text() {
						return latest.current?.text ?? "";
					},
					get title() {
						return latest.current?.title ?? "Copy";
					},
				};
	const enter = () => tools?.offer(id, self.current?.offsetTop ?? 0, live);
	const pointed = tools?.active === id;
	// In a bubble the sheet takes the keyboard's focus, since its keys are on
	// the bubble's toolbar: ⌘C (Ctrl+C) copies it while no words are
	// selected, and Enter opens a cut one.
	const keyboard =
		tools === null
			? {}
			: {
					tabIndex: 0,
					role: "group",
					"aria-label": KIND_NAMES[kind],
					onBlur: (event: FocusEvent<HTMLDivElement>) => {
						const to = event.relatedTarget as Node | null;
						if (to === null || (!self.current?.contains(to) && !(to instanceof Element && to.closest(".bubble-actions")))) tools.release(id);
					},
					onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => {
						if (event.target !== event.currentTarget) return;
						if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "c" && latest.current !== undefined && (window.getSelection()?.isCollapsed ?? true)) {
							event.preventDefault();
							void writeClipboard(latest.current.text);
						} else if (event.key === "Enter" && opens !== null) {
							event.preventDefault();
							opens();
						}
					},
				};
	return (
		<div
			ref={self}
			className="sheet"
			data-sheet={kind}
			data-bare={label === undefined || undefined}
			data-cut={opens !== null ? [cut?.wide && "wide", cut?.tall && "tall"].filter(Boolean).join(" ") : undefined}
			{...(tools !== null ? { onMouseEnter: enter, onFocus: enter } : {})}
			{...keyboard}
			{...(opens !== null ? { onClick: opens } : {})}
		>
			{(label !== undefined || tools === null) && (
				<div className="sheet-head">
					{label !== undefined && <span className="sheet-label">{label}</span>}
					{tools === null && (
						<span className="sheet-keys">
							{keys}
							{copy !== undefined && <CopyKey text={copy.text} title={copy.title} />}
						</span>
					)}
				</div>
			)}
			{tools !== null && pointed && tools.node !== null && createPortal(keys, tools.node)}
			{children}
			{opens !== null && (
				<button
					type="button"
					className="sheet-cut"
					title="Open"
					onClick={(event) => {
						event.stopPropagation();
						opens();
					}}
				>
					<span>{cut?.summary}</span>
					<ExpandIcon />
				</button>
			)}
		</div>
	);
}

/** What a sheet is, for a screen reader, when it takes the keyboard's focus in a bubble. */
const KIND_NAMES: Record<"table" | "code" | "diagram", string> = { table: "Table", code: "Code", diagram: "Diagram" };

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

