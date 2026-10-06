import type MermaidApi from "mermaid";
import { useEffect, useState, useSyncExternalStore } from "react";
import { Viewer } from "../ui/Viewer";

/**
 * A ```mermaid block drawn as the diagram it describes.
 *
 * The source comes out of a language model, so the drawing is shown as an
 * `<img>` of the SVG mermaid makes, never put into the page: an SVG loaded as
 * an image cannot run script, load anything or follow a link, so the worst a
 * hostile diagram can do is look wrong. Labels are plain SVG text for the same
 * reason (HTML labels would need `foreignObject`), and mermaid runs in its
 * strict mode on top.
 *
 * Mermaid is a few megabytes, so its script is loaded the first time a diagram
 * is on screen and never by a window that shows none. A block still streaming
 * in is not drawn until its source stops changing, and one that does not parse
 * stays the code it was, with a line saying so. A diagram drawn to fit a bubble
 * is often too small to read, so pressing it opens it in the viewer.
 */
export function Mermaid({ source }: { source: string }) {
	const theme = useResolvedTheme();
	const [drawn, setDrawn] = useState<{ key: string; url: string } | null>(null);
	const [failed, setFailed] = useState<string | null>(null);
	const [showSource, setShowSource] = useState(false);
	const [open, setOpen] = useState(false);
	const key = `${theme}\n${source}`;

	useEffect(() => {
		let gone = false;
		// Settle first: while a message streams, the source changes every few
		// characters and most of those prefixes are not diagrams yet. One
		// already drawn needs no settling.
		const timer = window.setTimeout(() => {
			void drawnUrl(source, theme).then(
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
	}, [key, source, theme]);

	const current = drawn?.key === key ? drawn : null;
	if (current === null || showSource) {
		return (
			<div className="mermaid-block">
				<pre>
					<code>{source}</code>
				</pre>
				{failed !== null && current === null && (
					<p className="mermaid-note" title={failed}>
						Couldn't draw this diagram, so here is its source.
					</p>
				)}
				{current !== null && (
					<button type="button" className="mermaid-toggle" onClick={() => setShowSource(false)}>
						Show diagram
					</button>
				)}
			</div>
		);
	}
	return (
		<div className="mermaid-block">
			<button type="button" className="picture-open" title="Open the diagram" onClick={() => setOpen(true)}>
				<img className="mermaid-diagram" src={current.url} alt="Diagram" draggable={false} />
			</button>
			<button type="button" className="mermaid-toggle" onClick={() => setShowSource(true)}>
				Show source
			</button>
			{open && <Viewer src={current.url} alt="Diagram" onClose={() => setOpen(false)} />}
		</div>
	);
}

const SETTLE_MS = 300;
/** How many drawings stay as URLs after their bubble leaves the screen. */
const KEPT = 40;
const diagrams = new Map<string, string>();

/**
 * The URL of a diagram, drawn once per theme and source and kept, so a
 * teammate switched back to shows theirs at once. The least recently shown is
 * let go when there are too many.
 */
async function drawnUrl(source: string, palette: Palette): Promise<string> {
	const key = `${palette}\n${source}`;
	const known = diagrams.get(key);
	if (known !== undefined) {
		diagrams.delete(key);
		diagrams.set(key, known);
		return known;
	}
	const svg = await draw(source, palette);
	const url = diagrams.get(key) ?? URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
	diagrams.set(key, url);
	if (diagrams.size > KEPT) {
		const oldest = diagrams.keys().next().value as string;
		URL.revokeObjectURL(diagrams.get(oldest)!);
		diagrams.delete(oldest);
	}
	return url;
}

type Palette = "light" | "dark";

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

function draw(source: string, palette: Palette): Promise<string> {
	const next = queue.then(async () => {
		const mermaid = await load();
		mermaid.initialize({
			startOnLoad: false,
			securityLevel: "strict",
			htmlLabels: false,
			flowchart: { htmlLabels: false },
			theme: palette === "dark" ? "dark" : "neutral",
			themeVariables: { fontFamily: FONT, background: "transparent" },
			fontFamily: FONT,
		});
		await mermaid.parse(source);
		const { svg } = await mermaid.render(`mermaid-${++drawn}`, source);
		return sized(svg);
	});
	queue = next.catch(() => {});
	return next;
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

/** The palette actually on screen, which on "System" is the OS's and can change while the window is open. */
function useResolvedTheme(): Palette {
	return useSyncExternalStore(
		(listener) => {
			const observer = new MutationObserver(listener);
			observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
			return () => observer.disconnect();
		},
		() => (document.documentElement.dataset.theme === "light" ? "light" : "dark"),
	);
}
