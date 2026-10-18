import { useEffect, useState } from "react";
import { appVersion, cancelUpdate, checkUpdate, installUpdate, openLink, updateStatus, watchUpdates, type UpdateStatus } from "../native";
import { Refusal, refused, type Refused } from "../ui/Refusal";
import { Markdown } from "./Markdown";

const RELEASES = "https://github.com/1Broseidon/hotline/releases/latest";
/**
 * The updater's own commands refuse with a label and its detail already
 * apart; anything else (a refusal from the shell, a plain sentence) is
 * labelled here.
 */
function refusal(label: string, error: unknown): Refused {
	if (typeof error === "object" && error !== null && !(error instanceof Error) && "message" in error && typeof error.message === "string") {
		return "detail" in error && typeof error.detail === "string" ? { message: error.message, detail: error.detail } : { message: error.message };
	}
	return refused(label, error);
}
const STATUS_UNREAD = "Couldn't read update status";
const mb = (bytes: number) => `${(bytes / 1_000_000).toFixed(1)} MB`;

export function UpdatesSection() {
	const [status, setStatus] = useState<UpdateStatus | null>(null);
	const [error, setError] = useState<Refused | null>(null);
	const [pending, setPending] = useState(false);
	useEffect(() => watchUpdates(setStatus, (error) => setError(refusal(STATUS_UNREAD, error))), []);

	const run = async (label: string, action: () => Promise<void>) => {
		setPending(true);
		setError(null);
		try { await action(); }
		catch (error) { setError(refusal(label, error)); }
		finally {
			try { setStatus(await updateStatus()); } catch (error) { setError(refusal(STATUS_UNREAD, error)); }
			setPending(false);
		}
	};

	const phase = status?.phase ?? "idle";
	const busy = pending || phase !== "idle";
	const available = status?.available;
	const downloading = phase === "downloading";
	const installing = phase === "installing" || phase === "restarting";
	const problem = error ?? status?.error;

	const checked = status?.checkedAt ? new Date(status.checkedAt * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) : null;
	const state =
		phase === "checking"
			? "Checking…"
			: available
				? `${available.version} is available`
				: (status?.disabledReason ?? (checked !== null && !problem ? `Up to date · checked ${checked}` : "Checks every six hours"));

	return (
		<section aria-label="Application updates">
			<div className="grouped">
				<div className="nt-fold-row">
					<span className="nt-fold-title">Hotline {status?.current || appVersion()}</span>
					<span className="nt-fold-value">{state}</span>
					<button type="button" className="nt-fold-action" disabled={busy || !status || !!status.disabledReason} onClick={() => void run("Update check failed", checkUpdate)}>
						{phase === "checking" ? "Checking…" : "Check now"}
					</button>
				</div>
				{available && (
					<div className="nt-fold-body pt-3">
						{available.notes && <div className="update-notes" aria-label="Release notes"><Markdown text={available.notes} /></div>}
						{downloading && <div className="flex flex-col gap-2">
							<progress className="w-full accent-[var(--accent)]" aria-label="Update download" max={status?.total ?? undefined} value={status?.total ? status.downloaded : undefined} />
							<p className="text-sm text-ink-2" role="status">Downloading {mb(status?.downloaded ?? 0)}{status?.total ? ` of ${mb(status.total)}` : ""}…</p>
						</div>}
						{installing && <p className="text-sm text-ink-2" role="status">{phase === "restarting" ? "Restarting Hotline…" : "Installing… Complete any system permission prompt to continue."}</p>}
						<div className="flex items-center gap-3">
							<button type="button" className="control btn btn-primary nt-submit" disabled={busy || !!status?.disabledReason} onClick={() => void run("Update failed", () => installUpdate(available.version))}>
								{installing ? "Updating…" : downloading ? "Downloading…" : "Update and restart"}
							</button>
							{downloading && <button type="button" className="control btn-quiet" onClick={() => void cancelUpdate().catch((error) => setError(refusal("Couldn't cancel download", error)))}>Cancel</button>}
							<span className="min-w-0 flex-1 text-sm text-ink-3">Waits for teammates to finish. Nothing is lost.</span>
						</div>
					</div>
				)}
				<div className="nt-fold-row">
					<span className="nt-fold-title">Release notes</span>
					<span className="nt-fold-value">{available ? `For ${available.version}, on GitHub` : "Every version, on GitHub"}</span>
					<button
						type="button"
						className="nt-fold-action"
						onClick={() => void openLink(available ? `https://github.com/1Broseidon/hotline/releases/tag/desktop-v${encodeURIComponent(available.version)}` : RELEASES)}
					>
						Open ↗
					</button>
				</div>
			</div>
			{problem && <Refusal {...problem} />}
		</section>
	);
}
