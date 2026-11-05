import { useEffect, useRef } from "react";
import { CloseIcon } from "../icons";
import { Markdown } from "./Markdown";
import { CopyKey, type FullObject, OpenFull } from "./Sheet";

const TITLES: Record<FullObject["kind"], string> = { table: "Table", code: "Code", diagram: "Diagram" };

/** What Copy takes from an object: a block of code without its fence, else its markdown as written. */
function copyText(object: FullObject): string {
	if (object.kind !== "code") return object.text;
	const lines = object.text.split("\n");
	return lines.slice(1, lines.at(-1)?.startsWith("```") ? -1 : undefined).join("\n");
}

/**
 * One object from a message as a pane of its own: a table, a block of code
 * or a diagram, drawn from its markdown by the same components the bubble
 * used, filling the conversation's place (and the side pane's) until it is
 * closed, by Escape or its close key. Nothing in it opens another.
 */
export function FullView({ object, onClose }: { object: FullObject; onClose(): void }) {
	const close = useRef<HTMLButtonElement>(null);
	useEffect(() => {
		close.current?.focus();
		const onKey = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			event.preventDefault();
			event.stopPropagation();
			onClose();
		};
		window.addEventListener("keydown", onKey, true);
		return () => window.removeEventListener("keydown", onKey, true);
	}, [onClose]);
	return (
		<section className="full-view" aria-label={object.title ?? TITLES[object.kind]}>
			<div className="full-view-frame" data-kind={object.kind}>
				<header className="full-view-head">
					<span className="full-view-title">{object.title ?? TITLES[object.kind]}</span>
					<CopyKey text={copyText(object)} title={object.kind === "code" ? "Copy the code" : "Copy as Markdown"} />
					<button ref={close} type="button" className="full-view-close" title="Close" aria-label="Close" onClick={onClose}>
						<CloseIcon />
					</button>
				</header>
				<div className="full-view-body">
					<OpenFull.Provider value={null}>
						<Markdown text={object.text} />
					</OpenFull.Provider>
				</div>
			</div>
		</section>
	);
}
