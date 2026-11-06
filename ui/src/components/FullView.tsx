import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { SourceIcon } from "../icons";
import { OpenBand, useSpeaker } from "../ui/OpenBand";
import { Markdown } from "./Markdown";
import { DiagramStage, unfenced } from "./Mermaid";
import { CopyKey, type FullObject, OpenFull, SheetKey } from "./Sheet";

/** What Copy takes from an object: a block of code without its fence, else its markdown as written. */
function copyText(object: FullObject): string {
	if (object.kind !== "code") return object.text;
	const lines = object.text.split("\n");
	return lines.slice(1, lines.at(-1)?.startsWith("```") ? -1 : undefined).join("\n");
}

/** The grow and the fade, as the tokens time them. */
const GROW_MS = 220;

const reduced = () => typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** The transform that lays `to` over `from`: where it starts growing, and where it goes back to. */
function over(from: DOMRect, to: DOMRect): string {
	if (to.width === 0 || to.height === 0) return "";
	return `translate(${from.left - to.left}px, ${from.top - to.top}px) scale(${from.width / to.width}, ${from.height / to.height})`;
}

/**
 * One object from a message, open in the conversation's place with the rail
 * beside it. It wears the band every pane does: ← back (Esc too), the
 * teammate's face and name and when they said it, and at its right the object
 * as a fact, a diagram's Source, and Copy. The sentence that introduced it
 * sits above it as a caption. A table or code is drawn at its own width from
 * the 24px inset, never stretched; a diagram stands on a stage of its own.
 *
 * It grows from the bubble's sheet to its place while the conversation fades
 * under it, and goes back the same way, the bubble scrolled into view first;
 * with reduced motion it crossfades. Focus goes back to the sheet it came from.
 */
export function FullView({ object, onClose }: { object: FullObject; onClose(): void }) {
	const view = useRef<HTMLElement>(null);
	const thing = useRef<HTMLDivElement>(null);
	const back = useRef<HTMLButtonElement>(null);
	const [state, setState] = useState<"opening" | "open" | "closing">("opening");
	const [showSource, setShowSource] = useState(false);
	const speaker = useSpeaker(object.from);
	const sheet = object.sheet?.isConnected ? object.sheet : undefined;

	// Opening: the object starts over the bubble's sheet and grows to its place, once.
	// biome-ignore lint/correctness/useExhaustiveDependencies: the grow runs once, from the sheet it opened from.
	useLayoutEffect(() => {
		const element = thing.current;
		const still = reduced();
		if (element !== null && sheet !== undefined && !still) {
			element.style.transition = "none";
			element.style.transform = over(sheet.getBoundingClientRect(), element.getBoundingClientRect());
			element.getBoundingClientRect();
			element.style.transition = "";
			element.style.transform = "";
		}
		// The closed look is laid out before it is let go, so the fade runs.
		view.current?.getBoundingClientRect();
		const timer = window.setTimeout(() => setState("open"), 0);
		back.current?.focus({ preventScroll: true });
		return () => window.clearTimeout(timer);
	}, []);

	const close = useCallback(() => {
		if (state === "closing") return;
		const element = thing.current;
		const done = () => {
			onClose();
			const target = sheet?.querySelector<HTMLElement>(".sheet-cut, .picture-open") ?? sheet;
			target?.focus({ preventScroll: true });
		};
		setState("closing");
		if (element === null || sheet === undefined || reduced()) {
			window.setTimeout(done, reduced() ? 130 : 0);
			return;
		}
		// The bubble comes back under the view first, so the object has a place to go.
		sheet.scrollIntoView({ block: "nearest" });
		element.style.transform = over(sheet.getBoundingClientRect(), element.getBoundingClientRect());
		window.setTimeout(done, GROW_MS);
	}, [state, sheet, onClose]);

	// While it is open, what it stands in front of is out of reach: Tab and a
	// screen reader stay in the view.
	useEffect(() => {
		const parent = view.current?.parentElement;
		if (!parent) return;
		const covered = [...parent.children].filter((child): child is HTMLElement => child !== view.current && child instanceof HTMLElement && !child.inert);
		for (const child of covered) child.inert = true;
		return () => {
			for (const child of covered) child.inert = false;
		};
	}, []);

	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			// A menu, a search or a dialog over the view takes its own Escape first.
			const over = document.querySelector('[role="dialog"][aria-modal="true"], [role="menu"]');
			if (over !== null && !view.current?.contains(over)) return;
			event.preventDefault();
			event.stopPropagation();
			close();
		};
		window.addEventListener("keydown", onKey, true);
		return () => window.removeEventListener("keydown", onKey, true);
	}, [close]);

	const diagram = object.kind === "diagram";
	return (
		<section ref={view} className="full-view" data-state={state} data-kind={object.kind} data-still={sheet === undefined || undefined} aria-label={object.fact}>
			<div className="full-view-ground" aria-hidden="true" />
			<div className="full-view-band">
				<OpenBand onBack={close} from={object.from ?? null} fact={object.fact} backRef={back}>
					{diagram && (
						<SheetKey
							icon={<SourceIcon />}
							label="Source"
							title={showSource ? "Show the diagram" : "Show the source"}
							pressed={showSource}
							onClick={() => setShowSource((was) => !was)}
						/>
					)}
					<CopyKey text={copyText(object)} title={object.kind === "code" ? "Copy the code" : "Copy as Markdown"} />
				</OpenBand>
			</div>
			<div className="full-view-scroll">
				<div className="full-view-body">
					{object.caption !== undefined && (
						<p className="full-view-caption">
							{speaker !== undefined && <span className="full-view-caption-who">{speaker.name}:</span>} {object.caption}
						</p>
					)}
					<div ref={thing} className="full-view-object">
						{diagram ? (
							showSource ? (
								<pre className="diagram-source">
									<code>{unfenced(object.text)}</code>
								</pre>
							) : (
								<DiagramStage source={unfenced(object.text)} />
							)
						) : (
							<OpenFull.Provider value={null}>
								<Markdown text={object.text} />
							</OpenFull.Provider>
						)}
					</div>
				</div>
			</div>
		</section>
	);
}
