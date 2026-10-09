import { useEffect, useState } from "react";
import { CloseIcon } from "../icons";
import { appVersion, watchUpdates, type UpdateStatus } from "../native";

const DISMISSED = "hotline.updateDismissed";

/** `VITE_PREVIEW_UPDATE=0.35.1 make dev` shows the card for that version; a dev build never checks for real. */
const PREVIEW = import.meta.env.DEV ? (import.meta.env.VITE_PREVIEW_UPDATE as string | undefined) : undefined;

function dismissedVersion(): string | null {
	try { return localStorage.getItem(DISMISSED); } catch { return null; }
}

/** The card's own rule, apart from the window: a version waiting, nothing under way, and not put away. */
export function updateToShow(status: UpdateStatus | null, dismissed: string | null): string | null {
	const available = status?.available;
	if (!available || status.disabledReason || status.phase !== "idle") return null;
	return available.version === dismissed ? null : available.version;
}

/**
 * A new version, said once in the window's corner: the six-hourly check
 * already found it, and Settings › Updates is where it is read and
 * installed. Closing the card puts that version away; a newer one brings
 * it back.
 */
export function UpdateFloat({ onOpen }: { onOpen: () => void }) {
	const [status, setStatus] = useState<UpdateStatus | null>(null);
	// A preview comes back on every launch; closing it lasts the run.
	const [dismissed, setDismissed] = useState(() => (PREVIEW ? null : dismissedVersion()));
	useEffect(() => (PREVIEW ? undefined : watchUpdates(setStatus, () => setStatus(null))), []);
	const shown: UpdateStatus | null = PREVIEW
		? { current: appVersion(), available: { version: PREVIEW, notes: "" }, checkedAt: null, phase: "idle", downloaded: 0, total: null, error: null, disabledReason: null }
		: status;
	const version = updateToShow(shown, dismissed);
	if (version === null) return null;
	const dismiss = () => {
		if (!PREVIEW) try { localStorage.setItem(DISMISSED, version); } catch { /* Private mode: put away for this run only. */ }
		setDismissed(version);
	};
	return (
		<aside className="update-float" aria-label="Update available">
			<div className="update-top">
				<span className="min-w-0 flex-1 font-medium">Update available</span>
				<button type="button" className="control btn-icon" aria-label="Not now" title="Not now" onClick={dismiss}>
					<CloseIcon />
				</button>
			</div>
			<p className="update-detail">Hotline {version} is available. Update now for the latest features and enhancements.</p>
			<button type="button" className="control btn-primary btn-sm self-start" onClick={onOpen}>Update…</button>
		</aside>
	);
}
