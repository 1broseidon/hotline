import { useEffect, useState } from "react";
import { appVersion, cancelUpdate, checkUpdate, installUpdate, openLink, updateStatus, watchUpdates, type UpdateStatus } from "../native";
import { Refusal } from "../ui/Refusal";

const RELEASES = "https://github.com/1Broseidon/hotline/releases/latest";
const describe = (error: unknown) => error instanceof Error ? error.message : String(error);
const mb = (bytes: number) => `${(bytes / 1_000_000).toFixed(1)} MB`;

export function UpdatesSection() {
	const [status, setStatus] = useState<UpdateStatus | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [pending, setPending] = useState(false);
	useEffect(() => watchUpdates(setStatus, (error) => setError(describe(error))), []);

	const run = async (action: () => Promise<void>) => {
		setPending(true);
		setError(null);
		try { await action(); }
		catch (error) { setError(describe(error)); }
		finally {
			try { setStatus(await updateStatus()); } catch (error) { setError(describe(error)); }
			setPending(false);
		}
	};

	const phase = status?.phase ?? "idle";
	const busy = pending || phase !== "idle";
	const available = status?.available;
	const downloading = phase === "downloading";
	const installing = phase === "installing" || phase === "restarting";
	const problem = error ?? status?.error;

	return (
		<section aria-label="Application updates">
			<h3 className="group-title">Hotline {status?.current || appVersion()}</h3>
			<div className="grouped">
				<div className="group-row">
					<div className="group-row-text">
						<span className="group-row-title">
							{phase === "checking" ? "Checking for updates…" : available ? `Version ${available.version} is available` : status?.checkedAt && !problem ? "You’re up to date" : "Application updates"}
						</span>
						<span className="group-row-detail">
							{status?.disabledReason ?? "Checks every six hours. You choose when to install."}
						</span>
					</div>
					<button type="button" className="control btn shrink-0" disabled={busy || !status || !!status.disabledReason} onClick={() => void run(checkUpdate)}>
						{phase === "checking" ? "Checking…" : "Check now"}
					</button>
				</div>
				{available && (
					<div className="flex flex-col gap-4 px-4 py-4">
						{available.notes && <div className="whitespace-pre-wrap text-sm text-ink-2" aria-label="Release notes">{available.notes}</div>}
						<button type="button" className="text-left text-sm text-accent hover:underline" onClick={() => void openLink(`https://github.com/1Broseidon/hotline/releases/tag/desktop-v${encodeURIComponent(available.version)}`)}>Full release notes ↗</button>
						{downloading && <div className="flex flex-col gap-2">
							<progress className="w-full accent-[var(--accent)]" aria-label="Update download" max={status?.total ?? undefined} value={status?.total ? status.downloaded : undefined} />
							<p className="text-sm text-ink-2" role="status">Downloading {mb(status?.downloaded ?? 0)}{status?.total ? ` of ${mb(status.total)}` : ""}…</p>
						</div>}
						{installing && <p className="text-sm text-ink-2" role="status">{phase === "restarting" ? "Restarting Hotline…" : "Installing… Complete any system permission prompt to continue."}</p>}
						<p className="text-sm text-ink-3">Teammates must finish their work before updating. Conversations, settings, and providers are kept.</p>
						<div className="flex items-center gap-2">
							<button type="button" className="control btn-primary" disabled={busy || !!status?.disabledReason} onClick={() => void run(() => installUpdate(available.version))}>
								{installing ? "Updating…" : downloading ? "Downloading…" : "Download, install and restart"}
							</button>
							{downloading && <button type="button" className="control btn" onClick={() => void cancelUpdate().catch((error) => setError(describe(error)))}>Cancel download</button>}
						</div>
					</div>
				)}
			</div>
			{status?.checkedAt && <p className="group-hint">Last checked {new Date(status.checkedAt * 1000).toLocaleString()}.</p>}
			{problem && <Refusal message={problem} />}
			{(!available || status?.disabledReason) && <button type="button" className="mt-3 text-sm text-accent hover:underline" onClick={() => void openLink(RELEASES)}>Open release page ↗</button>}
		</section>
	);
}
