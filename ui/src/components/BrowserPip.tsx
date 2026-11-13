import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { BrowserView, Viewport } from "../generated/contract";
import { useActiveDesk } from "../desks";
import { CloseIcon } from "../icons";
import { isDesktop } from "../native";
import { wire } from "../wire";
import { openRemoteBrowser } from "./RemoteBrowser";

/** How long after the teammate's last move the PIP stays up. */
const LINGER_MS = 45_000;

/**
 * A page size as people say it: the common ones by name (the sizes
 * `viewport` takes by name, and anything near them), any other as its
 * numbers. None for the desk's own.
 */
export function sizeName(viewport: Viewport | undefined): string | null {
	if (viewport === undefined) return null;
	const { width, height } = viewport;
	if (width <= 600) return "Phone";
	if (width <= 1100 && height > width) return "Tablet";
	if (width >= 1600) return "Wide";
	return `${width} × ${height}`;
}

/**
 * A teammate's desk browser, small, over the corner of its conversation:
 * up while the teammate is using it and for a while after, while the
 * person has it, and for as long as it waits on the person (`view.needs`,
 * as "Certificate expired"), which it marks in Needs-you amber. A click opens the browser pane, where the person watches
 * and can take control (crates/hotline-app/src/browser.rs); a browser on
 * another desk has no pane here, so the click opens the window's viewer of
 * it instead (RemoteBrowser). The picture is the desk's last snapshot,
 * fetched again whenever a newer one is taken.
 */
export function BrowserPip({ personaId, name, view }: { personaId: string; name: string; view: BrowserView | undefined }) {
	const [picture, setPicture] = useState<string>();
	const [dismissed, setDismissed] = useState<number>();
	const [lapsed, setLapsed] = useState<number>();
	const [refused, setRefused] = useState<string>();
	const remote = useActiveDesk()?.kind === "remote";

	// The PIP leaves when the teammate's last move is LINGER_MS old, and
	// nothing else here depends on the time: one timer, set for that moment,
	// and none while there is no browser or its linger is already over. A
	// window at rest is then not woken to ask whether anything changed.
	const activeAt = view?.activeAt;
	useEffect(() => {
		if (activeAt === undefined) return;
		const left = activeAt + LINGER_MS - Date.now();
		if (left <= 0) return;
		const timer = setTimeout(() => setLapsed(activeAt), left);
		return () => clearTimeout(timer);
	}, [activeAt]);

	const taken = view?.snapshotAt;
	useEffect(() => {
		if (taken === undefined) return;
		let gone = false;
		wire
			.command("browser.snapshot", { personaId })
			.then((snapshot) => {
				if (!gone) setPicture(`data:image/jpeg;base64,${snapshot.jpeg}`);
			})
			.catch(() => {});
		return () => {
			gone = true;
		};
	}, [personaId, taken]);

	if (view === undefined) return null;
	const yours = view.phase === "person";
	const size = sizeName(view.viewport);
	const shape = view.viewport === undefined ? "16 / 10" : `${view.viewport.width} / ${view.viewport.height}`;
	const upright = view.viewport !== undefined && view.viewport.height > view.viewport.width;
	const recent = lapsed !== view.activeAt && Date.now() - view.activeAt < LINGER_MS;
	const needs = view.needs;
	if (!yours && (dismissed === view.activeAt || (!needs && !recent))) return null;
	let host = view.url;
	try {
		host = new URL(view.url).host;
	} catch {}

	const open = () => {
		if (remote) {
			openRemoteBrowser(personaId, name);
			return;
		}
		if (!isDesktop()) return;
		setRefused(undefined);
		invoke("browser_pane_open", { personaId, name }).catch((reason) => setRefused(String(reason)));
	};

	return (
		<div
			className={`browser-pip absolute right-4 top-3 z-20 w-64 overflow-hidden rounded-xl border bg-raised ${needs ? "needs" : "border-line"}`}
		>
			<button
				type="button"
				className="block w-full text-left"
				onClick={open}
				title={needs ? "Needs you" : `Open ${name}'s browser`}
				aria-label={needs ? `Open ${name}'s browser: ${needs}` : `Open ${name}'s browser`}
			>
				<div className={`browser-pip-stage relative w-full bg-well${upright ? " upright" : ""}`} style={upright ? undefined : { aspectRatio: shape }}>
					{/* The page's own shape: a phone or a tablet stands upright in the middle. */}
					<div className="browser-pip-page" style={upright ? { aspectRatio: shape } : undefined}>
						{picture !== undefined && <img src={picture} alt="" className="h-full w-full object-cover object-top" draggable={false} />}
					</div>
					{size !== null && <span className="browser-pip-size">{size}</span>}
					{needs && (
						<span className="browser-pip-needs">
							<svg viewBox="0 0 16 16" aria-hidden="true">
								<path d="M8 1.8 15 14H1z" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" />
								<path d="M8 6.2v3.6M8 11.6v.4" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
							</svg>
							{needs}
						</span>
					)}
				</div>
				<div className="flex items-center gap-2 px-3 py-2 text-xs">
					<span className={`browser-pip-dot ${yours ? "yours" : needs ? "needs" : recent ? "live" : ""}`} />
					<span className="min-w-0 flex-1 truncate text-ink-2">
						{yours ? "You have control" : needs ? `${name} needs you` : `${name} is browsing`}{" "}
						<span className="text-ink-3">{host}</span>
					</span>
					{/* Room for Hide, which sits on the line rather than on the page, where a dark page swallowed it. */}
					{!yours && <span className="w-5 shrink-0" />}
				</div>
			</button>
			{!yours && (
				<button type="button" className="browser-pip-hide" onClick={() => setDismissed(view.activeAt)} title="Hide" aria-label="Hide">
					<CloseIcon />
				</button>
			)}
			{refused !== undefined && <p className="px-3 pb-2 text-xs text-ink-3">{refused}</p>}
		</div>
	);
}
