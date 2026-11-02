import { useEffect, useState } from "react";
import type { HarnessUpdate, HarnessUpdates } from "../generated/contract";
import { appVersion, cancelUpdate, checkUpdate, installUpdate, openLink, updateStatus, watchUpdates, type UpdateStatus } from "../native";
import { Refusal, refused, type Refused } from "../ui/Refusal";
import { wire } from "../wire";
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
			<HarnessSection />
		</section>
	);
}

const HARNESSES_UNREAD = "Couldn't check agent harnesses";
const at = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

/** What a harness row says of where it stands, from the desk's one state. */
export function harnessValue(harness: HarnessUpdate): string {
	switch (harness.state) {
		case "checking":
			return "Checking…";
		case "upToDate":
			return "Up to date";
		case "available":
			return harness.latest ? `${harness.latest} is available` : "Update available";
		case "unknown":
			return "Can't tell if it's current";
		case "updating":
			return "Updating…";
		case "updated":
			return "Updated";
		case "failed":
			return "Update failed";
	}
}

/**
 * Where the list stands. Each harness's own state is the desk's
 * (`HarnessUpdate.state`); this is only whether the list has arrived.
 */
type Listing = { k: "loading" } | { k: "ready"; updates: HarnessUpdates } | { k: "refused"; refusal: Refused };

/**
 * The agent harnesses this desk runs, one row each, whether the CLI, the
 * adapter that speaks ACP for it, or both are what is behind. Update runs on
 * the desk; a teammate on the harness gets it at its next start.
 */
export function HarnessSection() {
	const [listing, setListing] = useState<Listing>({ k: "loading" });
	const [checking, setChecking] = useState(false);

	const check = async () => {
		setChecking(true);
		try {
			setListing({ k: "ready", updates: await wire.command("harness.updates.check", {}) });
		} catch (error) {
			setListing({ k: "refused", refusal: refused(HARNESSES_UNREAD, error) });
		} finally {
			setChecking(false);
		}
	};

	// biome-ignore lint/correctness/useExhaustiveDependencies: read once, when the screen opens.
	useEffect(() => {
		let gone = false;
		void wire
			.command("harness.updates", {})
			.then((updates) => {
				if (gone) return;
				setListing({ k: "ready", updates });
				// Never looked at yet: look now rather than show an empty list.
				if (updates.checkedAt === undefined) void check();
			})
			.catch((error) => !gone && setListing({ k: "refused", refusal: refused(HARNESSES_UNREAD, error) }));
		return () => {
			gone = true;
		};
	}, []);

	const put = (row: HarnessUpdate) =>
		setListing((was) => (was.k === "ready" ? { k: "ready", updates: { ...was.updates, harnesses: was.updates.harnesses.map((one) => (one.backendId === row.backendId ? row : one)) } } : was));

	const update = async (harness: HarnessUpdate) => {
		const { detail: _detail, ...rest } = harness;
		put({ ...rest, state: "updating", canUpdate: false });
		try {
			put(await wire.command("harness.update", { backendId: harness.backendId }));
		} catch (error) {
			put({ ...harness, state: "failed", detail: error instanceof Error ? error.message : String(error) });
		}
	};

	const updates = listing.k === "ready" ? listing.updates : null;
	const busy = checking || updates?.harnesses.some((one) => one.state === "updating" || one.state === "checking") === true;
	return (
		<section className="grouped mt-4" aria-label="Agent harness updates">
			<div className="nt-fold-row">
				<span className="nt-fold-title">Agent harnesses</span>
				<span className="nt-fold-value">
					{checking || listing.k === "loading" ? "Checking…" : updates?.checkedAt !== undefined ? `Checked ${at(updates.checkedAt)}` : ""}
				</span>
				<button type="button" className="nt-fold-action" disabled={busy || listing.k === "loading"} onClick={() => void check()}>
					{checking ? "Checking…" : "Check now"}
				</button>
			</div>
			{updates?.harnesses.map((harness) => (
				<section key={harness.backendId} className="flex flex-col" aria-label={harness.name}>
					<div className="nt-fold-row">
						<span className="nt-fold-title">
							{harness.name}
							{harness.version ? ` ${harness.version}` : ""}
						</span>
						<span className="nt-fold-value">{harnessValue(harness)}</span>
						{harness.canUpdate && (
							<button type="button" className="nt-fold-action" disabled={busy} onClick={() => void update(harness)}>
								Update
							</button>
						)}
					</div>
					{harness.state === "failed" && harness.detail && <Refusal message="Update failed" detail={harness.detail} />}
					{harness.state === "updated" && harness.detail && (
						<p className="px-3 pb-2 text-sm text-ink-3" role="status">
							{harness.detail}
						</p>
					)}
				</section>
			))}
			{updates !== null && updates.harnesses.length === 0 && updates.checkedAt !== undefined && (
				<p className="px-3 py-2 text-sm text-ink-3">No agent harnesses are installed on this desk.</p>
			)}
			{listing.k === "refused" && <Refusal {...listing.refusal} />}
		</section>
	);
}
