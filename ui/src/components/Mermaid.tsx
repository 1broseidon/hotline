import { useEffect, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import type MermaidApi from "mermaid";
// mermaid's single-file build, served as a file of its own. Its ES module
// build splits into chunks that WebKitGTK's module loader rejects in
// development; this is one classic script that sets `globalThis.mermaid`,
// identical in development and in a release.
import MERMAID_URL from "mermaid/dist/mermaid.min.js?url";

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
 * is on screen and never by a window that shows none. A block still streaming in is
 * not drawn until its source stops changing, and one that does not parse stays
 * the code it was, with a line saying so.
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
			<button type="button" className="mermaid-open" aria-label="Open the diagram full size" onClick={() => setOpen(true)}>
				<img className="mermaid-diagram" src={current.url} alt="Diagram" draggable={false} />
			</button>
			<button type="button" className="mermaid-toggle" onClick={() => setShowSource(true)}>
				Show source
			</button>
			{open && <FullSize url={current.url} onClose={() => setOpen(false)} />}
		</div>
	);
}

/** The diagram at its own size over the window, scrolling when it is bigger. Escape or a click outside closes it. */
function FullSize({ url, onClose }: { url: string; onClose(): void }) {
	useEffect(() => {
		const close = (event: KeyboardEvent) => {
			if (event.key === "Escape") onClose();
		};
		window.addEventListener("keydown", close);
		return () => window.removeEventListener("keydown", close);
	}, [onClose]);
	return createPortal(
		<div
			className="mermaid-full"
			role="dialog"
			aria-label="Diagram"
			onClick={(event) => {
				if (event.target === event.currentTarget) onClose();
			}}
		>
			<img src={url} alt="Diagram" draggable={false} />
		</div>,
		document.body,
	);
}

const SETTLE_MS = 300;

type Palette = "light" | "dark";

let loading: Promise<typeof MermaidApi> | null = null;

function load(): Promise<typeof MermaidApi> {
	loading ??= new Promise((resolve, reject) => {
		const script = document.createElement("script");
		script.src = MERMAID_URL;
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
	});
	return loading;
}
let drawn = 0;

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
