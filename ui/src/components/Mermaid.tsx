import type MermaidApi from "mermaid";
import { type PointerEvent as ReactPointerEvent, type RefObject, useCallback, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { SourceIcon } from "../icons";
import { atRest, stir } from "../rest";
import { Sheet, SheetKey, useOpen } from "./Sheet";

/**
 * A ```mermaid block drawn as the diagram it describes, on its sheet.
 *
 * The source comes out of a language model, so the drawing is shown as an
 * `<img>` of the SVG mermaid makes, never put into the page: an SVG loaded as
 * an image cannot run script, load anything or follow a link, so the worst a
 * hostile diagram can do is look wrong. Labels are plain SVG text for the same
 * reason (HTML labels would need `foreignObject`), and mermaid runs in its
 * strict mode on top.
 *
 * Mermaid is a few megabytes, evaluating it holds the window up for longer
 * than a frame, and so does drawing one diagram. So neither is done as a
 * block scrolls into view: a block asks for its drawing as soon as it is in
 * the document, the transcript's rows measured off to the side included
 * (transcript/useRowWindow.ts), and the work is done while the window is at
 * rest (`ask`, below). By the time a row is scrolled to its diagram is drawn
 * and its height known. A window that shows no diagram never loads the
 * script: once run it holds some 50 MB, which the engine does not give
 * back when the script is let go, so it is not run on a guess.
 *
 * What stands in a diagram's place before it is drawn: its source, while
 * nobody knows how big the drawing will be; and once that is known (drawn
 * before, in another palette or since let go) an empty box of exactly the
 * drawing's size, so the picture arrives without moving anything. A block
 * still streaming in is not asked for until its source stops changing, and
 * one that does not parse stays the code it was, with a line saying so. A
 * diagram drawn to fit a bubble is often too small to read, so a click on
 * it shows it over the whole window, and Source shows what it was drawn
 * from.
 */
export function Mermaid({ source, caption }: { source: string; caption?: string | undefined }) {
	const look = useLook();
	// Read again whenever a drawing is made or one is found not to draw.
	useSyncExternalStore(onDrawings, () => made);
	const [showSource, setShowSource] = useState(false);
	const openFrom = useOpen();
	const key = `${look}\n${source}`;
	const block = useRef<HTMLDivElement>(null);
	const near = useNear(block);
	const drawing = pictures.get(key);
	const outcome = known.get(source);
	const size = drawing ?? (outcome !== undefined && "width" in outcome ? outcome : null);
	const failed = size !== null ? null : outcome !== undefined && "failed" in outcome ? outcome.failed : unloadable;

	useEffect(() => {
		if (drawing !== undefined) {
			if (near) shown(key);
			return;
		}
		// One that does not draw is not tried again; one whose size is known
		// is drawn again only to be looked at.
		if ((outcome !== undefined && "failed" in outcome) || (size !== null && !near)) return;
		// Settle first: while a message streams, the source changes every few
		// characters and most of those prefixes are not diagrams yet. A source
		// drawn before is whole. A block that leaves the document before its
		// turn no longer waits for one.
		let withdraw = () => {};
		const timer = window.setTimeout(
			() => {
				withdraw = ask(source, near);
			},
			size === null ? SETTLE_MS : 0,
		);
		return () => {
			window.clearTimeout(timer);
			withdraw();
		};
	}, [key, source, near, drawing, outcome, size]);

	const object = { kind: "diagram" as const, text: fenced(source), fact: factOf(source), caption };
	/* The drawing's own width and shape, so the box is its size before the picture has loaded (inline-objects.css). */
	const box = size === null ? undefined : { width: size.width, aspectRatio: `${size.width} / ${size.height}` };
	return (
		<Sheet
			kind="diagram"
			label={diagramKind(source)}
			keys={
				drawing !== undefined && (
					<SheetKey
						icon={<SourceIcon />}
						label="Source"
						title={showSource ? "Show the diagram" : "Show the source"}
						pressed={showSource}
						onClick={() => setShowSource((was) => !was)}
					/>
				)
			}
		>
			{/* `data-unsized`: this block is about to be another height, so the row it is in is not measured yet (useRowWindow.ts). */}
			<div ref={block} className="diagram-body" data-unsized={size === null && failed === null ? "" : undefined}>
				{size === null || showSource ? (
					<>
						<pre className="diagram-source">
							<code>{source}</code>
						</pre>
						{failed !== null && (
							<p className="mermaid-note" title={failed}>
								Couldn't draw this diagram, so here is its source.
							</p>
						)}
					</>
				) : drawing === undefined ? (
					<div className="mermaid-diagram" style={box} />
				) : openFrom !== null ? (
					<button type="button" className="picture-open" title="Open" onClick={() => openFrom(object, block.current?.closest<HTMLElement>(".sheet") ?? null)}>
						<img className="mermaid-diagram" style={box} src={drawing.url} alt="Diagram" draggable={false} />
					</button>
				) : (
					<img className="mermaid-diagram" style={box} src={drawing.url} alt="Diagram" draggable={false} />
				)}
			</div>
		</Sheet>
	);
}

/** What kind of diagram the source declares, as its head names it: `sequenceDiagram` is a sequence. */
function diagramKind(source: string): string {
	const declared = source
		.replace(/^---[\s\S]*?\n---\s*\n/, "")
		.split("\n")
		.map((line) => line.trim())
		.find((line) => line !== "" && !line.startsWith("%%"));
	const word = declared?.split(/\s+/)[0] ?? "";
	if (word === "graph" || word === "flowchart-elk") return "flowchart";
	return word.replace(/-v2$/, "").replace(/Diagram(-beta)?$/, "").replace(/-beta$/, "").toLowerCase() || "mermaid";
}

/** A diagram as the band says it: its kind, as a word ("Flowchart", "Sequence"). */
export function factOf(source: string): string {
	const kind = diagramKind(source);
	return kind.charAt(0).toUpperCase() + kind.slice(1);
}

/** The mermaid source inside a ```mermaid fence, as `fenced` wrote it. */
export function unfenced(text: string): string {
	const lines = text.split("\n");
	return lines.slice(1, /^`{3,}\s*$/.test(lines.at(-1) ?? "") ? -1 : undefined).join("\n");
}

/** How far a diagram zooms in and out, and by how much a key steps it. */
const ZOOM_MIN = 0.1;
const ZOOM_MAX = 4;
const ZOOM_STEP = 1.25;

/**
 * A diagram on the open view's stage: drawn at 1×, centred, and fitted down
 * only when it is larger than the stage, never up, since an enlarged
 * diagram's labels read like a toy's. Pinch or ⌘-scroll zooms about the
 * pointer, a drag pans, a double-click puts it back, and a quiet control at
 * the stage's bottom right says the scale while the pointer is on it.
 */
export function DiagramStage({ source }: { source: string }) {
	const look = useLook();
	const [url, setUrl] = useState<string | null>(null);
	const [failed, setFailed] = useState(false);
	const stage = useRef<HTMLDivElement>(null);
	const [size, setSize] = useState<{ w: number; h: number } | null>(null);
	const [view, setView] = useState<{ scale: number; x: number; y: number } | null>(null);
	const drag = useRef<{ x: number; y: number; from: { x: number; y: number } } | null>(null);

	// Drawn at once, not when the window is next at rest: the person has asked to see it.
	// biome-ignore lint/correctness/useExhaustiveDependencies: another look is another drawing, in the colours the window has by then.
	useEffect(() => {
		let gone = false;
		void load()
			.then((mermaid) => draw(mermaid, source, true))
			.then(
				(drawn) => !gone && drawn !== null && setUrl(drawn.url),
				() => !gone && setFailed(true),
			);
		return () => {
			gone = true;
		};
	}, [source, look]);

	/** The scale that fits the drawing in the stage: 1 when it fits as it is. */
	const fit = useCallback(() => {
		const box = stage.current;
		if (box === null || size === null) return 1;
		const room = { w: box.clientWidth - 48, h: box.clientHeight - 48 };
		return Math.min(1, room.w / size.w, room.h / size.h);
	}, [size]);
	const reset = useCallback(() => setView({ scale: fit(), x: 0, y: 0 }), [fit]);
	useLayoutEffect(() => {
		if (size !== null) reset();
	}, [size, reset]);

	const zoomTo = (scale: number, about?: { x: number; y: number }) =>
		setView((was) => {
			if (was === null) return was;
			const next = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, scale));
			if (about === undefined) return { ...was, scale: next };
			// Keep the point under the pointer where it is.
			const k = next / was.scale;
			return { scale: next, x: about.x - (about.x - was.x) * k, y: about.y - (about.y - was.y) * k };
		});

	// Pinch arrives as a wheel with ctrl held; ⌘ with the wheel zooms too. Not
	// passive, so the page does not zoom with it.
	useEffect(() => {
		const box = stage.current;
		if (box === null) return;
		const onWheel = (event: WheelEvent) => {
			if (!event.ctrlKey && !event.metaKey) return;
			event.preventDefault();
			const at = box.getBoundingClientRect();
			const about = { x: event.clientX - at.left - at.width / 2, y: event.clientY - at.top - at.height / 2 };
			setView((was) => {
				if (was === null) return was;
				const next = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, was.scale * Math.exp(-event.deltaY / 200)));
				const k = next / was.scale;
				return { scale: next, x: about.x - (about.x - was.x) * k, y: about.y - (about.y - was.y) * k };
			});
		};
		box.addEventListener("wheel", onWheel, { passive: false });
		return () => box.removeEventListener("wheel", onWheel);
	}, []);

	const onDown = (event: ReactPointerEvent<HTMLDivElement>) => {
		if (event.button !== 0 || view === null || (event.target as HTMLElement).closest(".zoom") !== null) return;
		drag.current = { x: event.clientX, y: event.clientY, from: { x: view.x, y: view.y } };
		event.currentTarget.setPointerCapture?.(event.pointerId);
	};
	const onMove = (event: ReactPointerEvent<HTMLDivElement>) => {
		const at = drag.current;
		if (at === null) return;
		setView((was) => (was === null ? was : { ...was, x: at.from.x + event.clientX - at.x, y: at.from.y + event.clientY - at.y }));
	};
	const onUp = () => {
		drag.current = null;
	};

	if (failed) return <p className="mermaid-note">Couldn't draw this diagram.</p>;
	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: the stage pans under a drag and zooms under a pinch; its control below does both from the keyboard.
		<div
			ref={stage}
			className="diagram-stage"
			data-grabbing={drag.current !== null || undefined}
			onPointerDown={onDown}
			onPointerMove={onMove}
			onPointerUp={onUp}
			onPointerCancel={onUp}
			onDoubleClick={reset}
		>
			{url !== null && (
				<img
					className="diagram-stage-drawing"
					src={url}
					alt="Diagram"
					draggable={false}
					onLoad={(event) => setSize({ w: event.currentTarget.naturalWidth, h: event.currentTarget.naturalHeight })}
					style={view === null ? { visibility: "hidden" } : { transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})` }}
				/>
			)}
			{view !== null && (
				<div className="zoom" role="toolbar" aria-label="Zoom">
					<button type="button" className="zoom-key" title="Zoom out" aria-label="Zoom out" onClick={() => zoomTo(view.scale / ZOOM_STEP)}>
						−
					</button>
					<span className="zoom-pct instrument">{Math.round(view.scale * 100)}%</span>
					<button type="button" className="zoom-key" title="Zoom in" aria-label="Zoom in" onClick={() => zoomTo(view.scale * ZOOM_STEP)}>
						+
					</button>
					<button type="button" className="zoom-key zoom-fit" title="Fit" onClick={reset}>
						Fit
					</button>
				</div>
			)}
		</div>
	);
}

/** The diagram as a ```mermaid block again, fenced longer than any run of backticks in it. */
function fenced(source: string): string {
	const longest = Math.max(2, ...[...source.matchAll(/`+/g)].map((run) => run[0].length));
	const fence = "`".repeat(longest + 1);
	return `${fence}mermaid\n${source}\n${fence}`;
}

const SETTLE_MS = 300;

/**
 * Whether the block has come within a screen of the scroller it sits in:
 * its drawing is then the next one made, and its picture is kept. It stays
 * true. The scroller is the observer's root because the window's viewport
 * would clip the block to the scroller and see it only once it was already
 * showing. A window without the observer takes every block for near.
 */
function useNear(block: RefObject<HTMLElement | null>): boolean {
	const [near, setNear] = useState(() => typeof IntersectionObserver === "undefined");
	useEffect(() => {
		const element = block.current;
		if (near || element === null) return;
		const observer = new IntersectionObserver(
			(entries) => {
				if (entries.some((entry) => entry.isIntersecting)) setNear(true);
			},
			{ root: element.closest(".scroller"), rootMargin: "100% 0px" },
		);
		observer.observe(element);
		return () => observer.disconnect();
	}, [block, near]);
	return near;
}

/** A drawing kept: its picture as a URL, and how big it is. */
type Drawing = { url: string; width: number; height: number };
/** What drawing a source came to: how big the drawing is, or why there is none. */
type Outcome = { width: number; height: number } | { failed: string };

/** How many pictures stay as URLs, by look (palette and accent) and source: the least recently shown is let go. */
const KEPT = 40;
/**
 * How many sources' outcomes are remembered. A size is two numbers, and it
 * is what lets a row stand at its height while its picture is made again,
 * so these are kept long after the pictures. A drawing is the same size in
 * every look: a look changes its colours and nothing else.
 */
const KNOWN = 400;
const pictures = new Map<string, Drawing>();
const known = new Map<string, Outcome>();
/** Why mermaid's script could not be loaded, until a later try loads it. */
let unloadable: string | null = null;

/* Counts every change to the three above: what a block reads them again for. */
let made = 0;
const watching = new Set<() => void>();
function changed(): void {
	made++;
	for (const listener of watching) listener();
}

/** Calls `listener` whenever a diagram is drawn or found not to draw. Returns what stops it. */
export function onDrawings(listener: () => void): () => void {
	watching.add(listener);
	return () => watching.delete(listener);
}

/** A picture on screen is the newest: the last to be let go. */
function shown(key: string): void {
	const drawing = pictures.get(key);
	if (drawing === undefined) return;
	pictures.delete(key);
	pictures.set(key, drawing);
}

/** A source waiting to be drawn, and how many blocks wait for it. `near` the view, its picture is kept whatever else is. */
type Wanted = { source: string; near: boolean; askers: number };
const line: Wanted[] = [];
/** A diagram is being drawn. */
let busy = false;

/**
 * Asks for a source to be drawn, in its turn: those near the view first,
 * the newest of them before the others. Returns what withdraws the asking,
 * for a block that leaves the document: a source nobody waits for is not
 * drawn.
 */
function ask(source: string, near: boolean): () => void {
	let wanted = line.find((one) => one.source === source);
	if (wanted === undefined) {
		wanted = { source, near: false, askers: 0 };
		line.push(wanted);
	}
	const asked = wanted;
	asked.askers++;
	if (near && !asked.near) {
		asked.near = true;
		line.splice(line.indexOf(asked), 1);
		line.unshift(asked);
	}
	work();
	return () => {
		asked.askers--;
		const at = line.indexOf(asked);
		if (asked.askers === 0 && at !== -1) line.splice(at, 1);
	};
}

/**
 * Does the next thing the line needs, once the window is at rest (rest.ts):
 * loads mermaid's script if it is not here, or draws one diagram. Each is a
 * turn of its own, and the end of one counts as a stir, so the window draws
 * frames and takes input between two. Every asking brings a turn; one that
 * finds a diagram being drawn, or nothing waiting, does nothing.
 */
function work(): void {
	if (line.length === 0) return;
	atRest(() => {
		const next = busy ? undefined : line[0];
		if (next === undefined) return;
		const mermaid = (globalThis as { mermaid?: typeof MermaidApi }).mermaid;
		if (mermaid === undefined) {
			// The line stays as it is: whoever asks next tries the script again.
			load().then(
				() => {
					if (unloadable !== null) {
						unloadable = null;
						changed();
					}
					stir();
					work();
				},
				(error: unknown) => {
					unloadable = error instanceof Error ? error.message : String(error);
					changed();
				},
			);
			return;
		}
		line.shift();
		busy = true;
		const done = () => {
			busy = false;
			stir();
			work();
		};
		// What it came to is in `known`, put there by `draw`.
		draw(mermaid, next.source, next.near).then(done, done);
	});
}

/**
 * mermaid's single-file build, as a classic script that sets
 * `globalThis.mermaid`. Its ES module build splits into chunks WebKitGTK's
 * module loader rejects in development; this one file behaves the same in
 * development and in a release. Its URL is imported here, when a diagram is
 * first drawn, so nothing runs it at import time. The tag in the page is
 * the load on its way: whoever else asks meanwhile waits on the same one.
 */
function load(): Promise<typeof MermaidApi> {
	const here = () => (globalThis as { mermaid?: typeof MermaidApi }).mermaid;
	const loaded = here();
	if (loaded !== undefined) return Promise.resolve(loaded);
	return import("mermaid/dist/mermaid.min.js?url").then(
		({ default: url }) =>
			new Promise<typeof MermaidApi>((resolve, reject) => {
				const arrived = here();
				if (arrived !== undefined) {
					resolve(arrived);
					return;
				}
				let script = document.head.querySelector<HTMLScriptElement>("script[data-mermaid]");
				if (script === null) {
					script = document.createElement("script");
					script.dataset["mermaid"] = "";
					script.src = url;
					script.async = true;
					document.head.append(script);
				}
				const tag = script;
				// A tag that came to nothing is taken out, so the next try loads it afresh.
				const fail = (why: string) => {
					tag.remove();
					reject(new Error(why));
				};
				tag.addEventListener("load", () => {
					const mermaid = here();
					if (mermaid !== undefined) resolve(mermaid);
					else fail("mermaid did not load");
				});
				tag.addEventListener("error", () => fail("mermaid could not be loaded"));
			}),
	);
}

/** One render at a time: mermaid keeps global configuration, and the theme is part of it. */
let queue: Promise<unknown> = Promise.resolve();
let drawn = 0;

/**
 * Draws a source in the look the window has now, and remembers what it came
 * to: its size, or that it does not draw. The picture is kept when it is
 * `wanted` (its block is near the view, or open) or there is room for it;
 * one drawn ahead of the view never takes the place of a picture already
 * kept, since those were drawn nearer the view than it. Resolves to the
 * picture when one is kept.
 */
function draw(mermaid: typeof MermaidApi, source: string, wanted: boolean): Promise<Drawing | null> {
	const next = queue.then(async () => {
		const key = `${lookNow()}\n${source}`;
		const kept = pictures.get(key);
		if (kept !== undefined) return kept;
		try {
			const colours = tokenColours();
			mermaid.initialize(mermaidConfig(colours));
			const marked = lit(source, colours);
			await mermaid.parse(marked);
			const { svg } = await mermaid.render(`mermaid-${++drawn}`, marked);
			const picture = sized(svg);
			remember(source, { width: picture.width, height: picture.height });
			if (!wanted && pictures.size >= KEPT) return null;
			const drawing = { url: URL.createObjectURL(new Blob([picture.svg], { type: "image/svg+xml" })), width: picture.width, height: picture.height };
			pictures.set(key, drawing);
			if (pictures.size > KEPT) {
				const oldest = pictures.keys().next().value as string;
				URL.revokeObjectURL(pictures.get(oldest)!.url);
				pictures.delete(oldest);
			}
			return drawing;
		} catch (error) {
			remember(source, { failed: error instanceof Error ? error.message : String(error) });
			throw error;
		} finally {
			changed();
		}
	});
	queue = next.catch(() => {});
	return next;
}

function remember(source: string, outcome: Outcome): void {
	known.delete(source);
	known.set(source, outcome);
	if (known.size > KNOWN) known.delete(known.keys().next().value as string);
}

/** The window's tokens a diagram is drawn in, as colours an SVG image and mermaid's colour maths can read. */
export type DiagramColours = {
	dark: boolean;
	ink: string;
	ink3: string;
	line: string;
	lineStrong: string;
	canvas: string;
	raised: string;
	pop: string;
	accent: string;
	accentSoft: string;
};

/**
 * Mermaid's configuration: its neutral base theme re-valued from the
 * window's tokens, so a diagram is drawn in the desk's planes and inks in
 * either palette. Nothing here is the accent: a node is lit only where the
 * source marks it (`lit`).
 */
export function mermaidConfig(colours: DiagramColours) {
	const { ink, ink3, line, lineStrong, canvas, raised, pop } = colours;
	return {
		startOnLoad: false,
		securityLevel: "strict" as const,
		htmlLabels: false,
		theme: "base" as const,
		darkMode: colours.dark,
		fontFamily: FONT,
		themeVariables: {
			darkMode: colours.dark,
			background: "transparent",
			// Nothing in the window rests on a shadow but the composer (ui/design.md).
			dropShadow: "none",
			fontFamily: FONT,
			fontSize: "13px",
			primaryColor: pop,
			primaryTextColor: ink,
			primaryBorderColor: lineStrong,
			secondaryColor: raised,
			secondaryTextColor: ink,
			secondaryBorderColor: lineStrong,
			tertiaryColor: raised,
			tertiaryTextColor: ink,
			tertiaryBorderColor: line,
			mainBkg: pop,
			nodeBorder: lineStrong,
			nodeTextColor: ink,
			textColor: ink,
			titleColor: ink,
			lineColor: ink3,
			edgeLabelBackground: canvas,
			clusterBkg: raised,
			clusterBorder: line,
			noteBkgColor: raised,
			noteTextColor: ink,
			noteBorderColor: line,
			actorBkg: pop,
			actorBorder: lineStrong,
			actorTextColor: ink,
			actorLineColor: ink3,
			signalColor: ink3,
			signalTextColor: ink,
			labelBoxBkgColor: pop,
			labelBoxBorderColor: lineStrong,
			labelTextColor: ink,
			loopTextColor: ink,
		},
	};
}

/* A `classDef` (other than `default`, which is every node) or a `style` line:
 * what a source says to mark a node. */
const MARKING = /^(\s*(?:classDef\s+(?!default\b)[\w,-]+|style\s+[\w,-]+)\s+)\S.*$/gm;

/**
 * The source with every node it marks lit in the accent, and no other. A
 * model picks its own colours for a highlighted node, which would clash with
 * the window's; the marking is what it meant, so the marking is kept and the
 * colours are the desk's. A diagram that marks nothing is drawn plain.
 */
export function lit(source: string, colours: DiagramColours): string {
	return source.replace(MARKING, (_line, head: string) => `${head}fill:${colours.accentSoft},stroke:${colours.accent},color:${colours.ink}`);
}

/**
 * The tokens as hex, read off the window as it is drawn now. Mermaid's colour
 * maths cannot parse oklch, and a classDef's comma-separated styles cannot
 * hold an rgba(), so each is painted onto a pixel and read back: #rrggbb, or
 * #rrggbbaa where the token is translucent.
 */
function tokenColours(): DiagramColours {
	const probe = document.createElement("i");
	document.body.append(probe);
	const canvas = document.createElement("canvas");
	canvas.width = 1;
	canvas.height = 1;
	const paint = canvas.getContext("2d", { willReadFrequently: true });
	const read = (token: string): string => {
		probe.style.color = `var(--${token})`;
		if (paint === null) return getComputedStyle(probe).color;
		paint.clearRect(0, 0, 1, 1);
		paint.fillStyle = getComputedStyle(probe).color;
		paint.fillRect(0, 0, 1, 1);
		const [r = 0, g = 0, b = 0, a = 255] = paint.getImageData(0, 0, 1, 1).data;
		return `#${[r, g, b, ...(a < 255 ? [a] : [])].map((channel) => channel.toString(16).padStart(2, "0")).join("")}`;
	};
	const colours: DiagramColours = {
		dark: document.documentElement.dataset["theme"] !== "light",
		ink: read("ink"),
		ink3: read("ink-3"),
		line: read("line"),
		lineStrong: read("line-strong"),
		canvas: read("canvas"),
		raised: read("raised"),
		pop: read("pop"),
		accent: read("accent"),
		accentSoft: read("accent-soft"),
	};
	probe.remove();
	return colours;
}

/** A font the image can reach: it cannot see the window's web fonts, so it measures and draws in the system's. */
const FONT = "ui-sans-serif, system-ui, sans-serif";

/**
 * Mermaid sizes its SVG to its container (`width="100%"`, a max-width style),
 * which an image has none of. The viewBox is the drawing's real size, so that
 * becomes its width and height, and is what the block lays its box out by.
 */
function sized(svg: string): { svg: string; width: number; height: number } {
	const document = new DOMParser().parseFromString(svg, "image/svg+xml");
	const root = document.documentElement;
	const box = root.getAttribute("viewBox")?.split(/[\s,]+/).map(Number);
	if (box?.length !== 4 || !box.every(Number.isFinite)) throw new Error("mermaid drew a diagram with no size");
	const width = Math.ceil(box[2] ?? 0);
	const height = Math.ceil(box[3] ?? 0);
	root.setAttribute("width", String(width));
	root.setAttribute("height", String(height));
	root.removeAttribute("style");
	return { svg: new XMLSerializer().serializeToString(document), width, height };
}

/**
 * The look actually on screen: the palette, which on "System" is the OS's and
 * can change while the window is open, and the room's accent.
 */
function lookNow(): string {
	const { theme, accent } = document.documentElement.dataset;
	return `${theme === "light" ? "light" : "dark"} ${accent ?? ""}`;
}

function useLook(): string {
	return useSyncExternalStore((listener) => {
		const observer = new MutationObserver(listener);
		observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-accent"] });
		return () => observer.disconnect();
	}, lookNow);
}
