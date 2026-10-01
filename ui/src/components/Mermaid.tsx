import { useEffect, useState, useSyncExternalStore } from "react";

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
 * Mermaid is a few megabytes, so it is fetched the first time a diagram is on
 * screen and never by a window that shows none. A block still streaming in is
 * not drawn until its source stops changing, and one that does not parse stays
 * the code it was, with a line saying so.
 */
export function Mermaid({ source }: { source: string }) {
	const theme = useResolvedTheme();
	const [drawn, setDrawn] = useState<{ key: string; url: string } | null>(null);
	const [failed, setFailed] = useState<string | null>(null);
	const [showSource, setShowSource] = useState(false);
	const key = `${theme}\n${source}`;

	useEffect(() => {
		let gone = false;
		let url: string | null = null;
		// Settle first: while a message streams, the source changes every few
		// characters and most of those prefixes are not diagrams yet.
		const timer = window.setTimeout(() => {
			void draw(source, theme).then(
				(svg) => {
					if (gone) return;
					url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
					setDrawn({ key, url });
					setFailed(null);
				},
				(error: unknown) => {
					if (!gone) setFailed(error instanceof Error ? error.message : String(error));
				},
			);
		}, SETTLE_MS);
		return () => {
			gone = true;
			window.clearTimeout(timer);
			if (url !== null) URL.revokeObjectURL(url);
		};
	}, [key, source, theme]);

	const current = drawn?.key === key ? drawn : null;
	if (current === null || showSource) {
		return (
			<div className="mermaid-block">
				<pre>
					<code>{source}</code>
				</pre>
				{failed !== null && current === null && <p className="mermaid-note">Couldn't draw this diagram, so here is its source.</p>}
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
			<img className="mermaid-diagram" src={current.url} alt="Diagram" draggable={false} />
			<button type="button" className="mermaid-toggle" onClick={() => setShowSource(true)}>
				Show source
			</button>
		</div>
	);
}

const SETTLE_MS = 300;

type Palette = "light" | "dark";

let loading: Promise<typeof import("mermaid").default> | null = null;
let drawn = 0;

/** One render at a time: mermaid keeps global configuration, and the theme is part of it. */
let queue: Promise<unknown> = Promise.resolve();

function draw(source: string, palette: Palette): Promise<string> {
	const next = queue.then(async () => {
		loading ??= import("mermaid").then((module) => module.default);
		const mermaid = await loading;
		mermaid.initialize({
			startOnLoad: false,
			securityLevel: "strict",
			htmlLabels: false,
			flowchart: { htmlLabels: false },
			theme: palette === "dark" ? "dark" : "neutral",
			themeVariables: {
				fontFamily: FONT,
				background: "transparent",
			},
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
