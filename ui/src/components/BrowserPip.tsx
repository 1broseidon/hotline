import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { BrowserView } from "../generated/contract";
import { isDesktop } from "../native";
import { wire } from "../wire";

/** How long after the teammate's last move the PIP stays up. */
const LINGER_MS = 45_000;

/**
 * A teammate's desk browser, small, over the corner of its conversation:
 * up while the teammate is using it and for a while after, while the
 * person has it, and for as long as it waits on the person (`view.needs`,
 * as "Certificate expired"), which it marks in Needs-you amber. A click opens the browser pane, where the person watches
 * and can take control (crates/hotline-app/src/browser.rs). The picture is
 * the desk's last snapshot, fetched again whenever a newer one is taken.
 */
export function BrowserPip({ personaId, name, view }: { personaId: string; name: string; view: BrowserView | undefined }) {
	const [picture, setPicture] = useState<string>();
	const [dismissed, setDismissed] = useState<number>();
	const [now, setNow] = useState(() => Date.now());
	const [refused, setRefused] = useState<string>();

	useEffect(() => {
		const timer = setInterval(() => setNow(Date.now()), 5_000);
		return () => clearInterval(timer);
	}, []);

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
	const recent = now - view.activeAt < LINGER_MS;
	const needs = view.needs;
	if (!yours && (dismissed === view.activeAt || (!needs && !recent))) return null;
	let host = view.url;
	try {
		host = new URL(view.url).host;
	} catch {}

	const open = () => {
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
				<div className="relative aspect-[16/10] w-full bg-well">
					{picture !== undefined && <img src={picture} alt="" className="h-full w-full object-cover object-top" draggable={false} />}
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
				</div>
			</button>
			{!yours && (
				<button
					type="button"
					className="absolute right-1.5 top-1.5 grid h-6 w-6 place-items-center rounded-full bg-raised/80 text-ink-3 hover:text-ink"
					onClick={() => setDismissed(view.activeAt)}
					title="Hide"
					aria-label="Hide"
				>
					×
				</button>
			)}
			{refused !== undefined && <p className="px-3 pb-2 text-xs text-ink-3">{refused}</p>}
		</div>
	);
}
