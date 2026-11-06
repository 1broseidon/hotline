import type MermaidApi from "mermaid";
import { type PointerEvent as ReactPointerEvent, type RefObject, useCallback, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { SourceIcon } from "../icons";
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
 * Mermaid is a few megabytes, so its script is loaded the first time a diagram
 * is on screen and never by a window that shows none. A long conversation can
 * hold dozens of diagrams, and drawing them all on opening it would hold the
 * window up for rows nobody is looking at, so a block is drawn once it comes
 * within a screen of being seen; one drawn before is shown at once. A block
 * still streaming in is not drawn until its source stops changing, and one
 * that does not parse stays the code it was, with a line saying so. A diagram
 * drawn to fit a bubble is often too small to read, so a click on it shows it
 * over the whole window, and Source shows what it was drawn from.
 */
export function Mermaid({ source, caption }: { source: string; caption?: string | undefined }) {
	const look = useLook();
	const [drawn, setDrawn] = useState<{ key: string; url: string } | null>(null);
	const [failed, setFailed] = useState<string | null>(null);
	const [showSource, setShowSource] = useState(false);
	const openFrom = useOpen();
	const key = `${look}\n${source}`;
	const block = useRef<HTMLDivElement>(null);
	const near = useNear(block);

	useEffect(() => {
		if (!near && !diagrams.has(key)) return;
		let gone = false;
		// Settle first: while a message streams, the source changes every few
		// characters and most of those prefixes are not diagrams yet. One
		// already drawn needs no settling.
		const timer = window.setTimeout(() => {
			void drawnUrl(source, look).then(
				(url) => {
					if (gone) return;
					setDrawn({ key, url });
					setFailed(null);
				},
				(error: unknown) => {
					if (!gone) setFailed(error instanceof Error ? error.message : String(error));
				},
			);
		}, diagrams.has(key) ? 0 : SETTLE_MS);
		return () => {
			gone = true;
			window.clearTimeout(timer);
		};
	}, [key, source, look, near]);

	const current = drawn?.key === key ? drawn : null;
	const object = { kind: "diagram" as const, text: fenced(source), fact: factOf(source), caption };
	return (
		<Sheet
			kind="diagram"
			label={diagramKind(source)}
			keys={
				current !== null && (
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
			<div ref={block} className="diagram-body">
				{current === null || showSource ? (
					<>
						<pre className="diagram-source">
							<code>{source}</code>
						</pre>
						{failed !== null && current === null && (
							<p className="mermaid-note" title={failed}>
								Couldn't draw this diagram, so here is its source.
							</p>
						)}
					</>
				) : openFrom !== null ? (
					<button type="button" className="picture-open" title="Open" onClick={() => openFrom(object, block.current?.closest<HTMLElement>(".sheet") ?? null)}>
						<img className="mermaid-diagram" src={current.url} alt="Diagram" draggable={false} />
					</button>
				) : (
					<img className="mermaid-diagram" src={current.url} alt="Diagram" draggable={false} />
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

	useEffect(() => {
		let gone = false;
		void drawnUrl(source, look).then(
			(drawn) => !gone && setUrl(drawn),
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
 * Whether the block has come within a screen of the scroller it sits in. It
 * stays true: a diagram drawn once is not taken down when it scrolls away.
 * The scroller is the observer's root because the window's viewport would
 * clip the block to the scroller and see it only once it was already showing.
 * A window without the observer draws everything, as it did before.
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
/** How many drawings stay as URLs after their bubble leaves the screen. */
const KEPT = 40;
const diagrams = new Map<string, string>();

/**
 * The URL of a diagram, drawn once per look (palette and accent) and source
 * and kept, so a teammate switched back to shows theirs at once. The least
 * recently shown is let go when there are too many.
 */
async function drawnUrl(source: string, look: string): Promise<string> {
	const key = `${look}\n${source}`;
	const known = diagrams.get(key);
	if (known !== undefined) {
		diagrams.delete(key);
		diagrams.set(key, known);
		return known;
	}
	const svg = await draw(source);
	const url = diagrams.get(key) ?? URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
	diagrams.set(key, url);
	if (diagrams.size > KEPT) {
		const oldest = diagrams.keys().next().value as string;
		URL.revokeObjectURL(diagrams.get(oldest)!);
		diagrams.delete(oldest);
	}
	return url;
}

let loading: Promise<typeof MermaidApi> | null = null;
let drawn = 0;

/**
 * mermaid's single-file build, as a classic script that sets
 * `globalThis.mermaid`. Its ES module build splits into chunks WebKitGTK's
 * module loader rejects in development; this one file behaves the same in
 * development and in a release. Its URL is imported here, when a diagram is
 * first drawn, so nothing runs it at import time.
 */
function load(): Promise<typeof MermaidApi> {
	loading ??= import("mermaid/dist/mermaid.min.js?url").then(
		({ default: url }) =>
			new Promise<typeof MermaidApi>((resolve, reject) => {
				const script = document.createElement("script");
				script.src = url;
				script.async = true;
				script.onload = () => {
					const loaded = (globalThis as { mermaid?: typeof MermaidApi }).mermaid;
					if (loaded) resolve(loaded);
					else reject(new Error("mermaid did not load"));
				};
				script.onerror = () => {
					loading = null;
					reject(new Error("mermaid could not be loaded"));
				};
				document.head.append(script);
			}),
	);
	return loading;
}

/** One render at a time: mermaid keeps global configuration, and the theme is part of it. */
let queue: Promise<unknown> = Promise.resolve();

function draw(source: string): Promise<string> {
	const next = queue.then(async () => {
		const mermaid = await load();
		const colours = tokenColours();
		mermaid.initialize(mermaidConfig(colours));
		const marked = lit(source, colours);
		await mermaid.parse(marked);
		const { svg } = await mermaid.render(`mermaid-${++drawn}`, marked);
		return sized(svg);
	});
	queue = next.catch(() => {});
	return next;
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
 * becomes its width and height.
 */
function sized(svg: string): string {
	const document = new DOMParser().parseFromString(svg, "image/svg+xml");
	const root = document.documentElement;
	const box = root.getAttribute("viewBox")?.split(/[\s,]+/).map(Number);
	if (box?.length === 4 && box.every(Number.isFinite)) {
		root.setAttribute("width", String(Math.ceil(box[2] ?? 0)));
		root.setAttribute("height", String(Math.ceil(box[3] ?? 0)));
		root.removeAttribute("style");
	}
	return new XMLSerializer().serializeToString(document);
}

/**
 * The look actually on screen: the palette, which on "System" is the OS's and
 * can change while the window is open, and the room's accent.
 */
function useLook(): string {
	return useSyncExternalStore(
		(listener) => {
			const observer = new MutationObserver(listener);
			observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-accent"] });
			return () => observer.disconnect();
		},
		() => {
			const { theme, accent } = document.documentElement.dataset;
			return `${theme === "light" ? "light" : "dark"} ${accent ?? ""}`;
		},
	);
}
