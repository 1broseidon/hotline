import { Fragment, useEffect, useState } from "react";
import type { ComputerReleases, ComputerRuntime, RuntimeReport, RuntimeState, CookieOfferSettings as OfferSettings } from "../../generated/contract";
import { openLink, pinnedComputerImage } from "../../native";
import { InfoIcon } from "../../icons";
import { Fold, toggled } from "../../ui/Fold";
import { Picker } from "../../ui/Menu";
import { wire } from "../../wire";
import { CookieOfferSettings } from "./CookieOfferSettings";

const RUNTIME_NAMES: Record<ComputerRuntime, string> = {
	docker: "Docker",
	podman: "Podman",
	container: "Apple container",
};

/** Two words per state; the runtime's own words go behind the info key. */
const STATE_NAMES: Record<Exclude<RuntimeState, "ready">, string> = {
	not_installed: "Not installed",
	not_running: "Not running",
	not_responding: "Not responding",
	failed: "Failed",
	unsupported: "macOS only",
};

/** What to do about a runtime that is not ready, and where it is written up. */
const RUNTIME_HELP: Record<ComputerRuntime, { install: string; start: string; docs: string }> = {
	docker: { install: "Install Docker Desktop or OrbStack.", start: "Start Docker Desktop or OrbStack.", docs: "https://docs.docker.com/get-started/get-docker/" },
	podman: { install: "Install Podman.", start: "Run podman machine start.", docs: "https://podman.io/docs/installation" },
	container: { install: "Install Apple container.", start: "Run container system start.", docs: "https://github.com/apple/container" },
};

function runtimeAdvice(report: RuntimeReport): string | null {
	const help = RUNTIME_HELP[report.runtime];
	switch (report.state) {
		case "not_installed":
			return help.install;
		case "not_running":
			return help.start;
		case "not_responding":
		case "failed":
			return `Run ${report.runtime} version in a terminal.`;
		default:
			return null;
	}
}

/** The Release picker's entry that opens the free-text image field. */
const CUSTOM_IMAGE = "custom";
/** Where a teammate's computer is built and explained. */
const COMPUTER_REPO = "https://github.com/1broseidon/hotline-computer";

/** A wall-clock time for the strip, in the viewer's own locale. */
function clock(ms: number): string {
	return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/**
 * Computer: which container runtime wakes a teammate's desktop, and which
 * image it wakes. Every runtime this machine could have is a row, the
 * missing ones greyed with two words for how, and an info key that opens
 * what the runtime said and what to do about it, so "no computer" is never
 * a mystery and never a paragraph. Automatic is the room's default and
 * leaves the pick to the desk, rootless first.
 */
export function ComputerSection({
	runtime,
	image,
	onRuntime,
	onImage,
	cookieOffers,
	onSites,
}: {
	runtime: string | null;
	image: string | null;
	onRuntime(id: string | null): void;
	onImage(image: string | null): void;
	cookieOffers: OfferSettings;
	onSites(): void;
}) {
	const [reports, setReports] = useState<RuntimeReport[] | undefined>(undefined);
	const [releases, setReleases] = useState<ComputerReleases | null>(null);
	const [draft, setDraft] = useState(image ?? "");
	const [shown, setShown] = useState<ComputerRuntime | null>(null);
	// The Release picker: Newest, a published release, or a custom image
	// typed in. Custom stays open while the field is being written in, even
	// though the room still says Newest until it is committed.
	const [custom, setCustom] = useState(false);
	const [checking, setChecking] = useState(false);

	useEffect(() => {
		setDraft(image ?? "");
	}, [image]);

	const checkReleases = async () => {
		if (checking) return;
		setChecking(true);
		try {
			setReleases(await wire.command("computer.releases.check", {}));
		} catch (error) {
			setReleases((known) => ({
				floor: known?.floor ?? "",
				repository: known?.repository ?? "",
				releases: known?.releases ?? [],
				...(known?.newest !== undefined ? { newest: known.newest } : {}),
				checkedAt: Date.now(),
				error: error instanceof Error ? error.message : String(error),
			}));
		} finally {
			setChecking(false);
		}
	};

	// What the picker says the room is on: a release tag when the image is
	// one of the repository's, Newest when blank, else the custom image.
	const pickedRelease = (() => {
		if (custom) return CUSTOM_IMAGE;
		if (image === null) return "";
		const prefix = releases !== null ? `${releases.repository}:` : null;
		if (prefix !== null && image.startsWith(prefix) && releases?.releases.includes(image.slice(prefix.length))) {
			return image.slice(prefix.length);
		}
		return CUSTOM_IMAGE;
	})();
	const releaseChoices = [
		{ id: "", name: releases?.newest !== undefined ? `Newest · ${releases.newest}` : "Newest" },
		...(releases?.releases ?? []).map((tag) => ({ id: tag, name: tag })),
		{ id: CUSTOM_IMAGE, name: "Custom image…" },
	];
	const pickRelease = (id: string) => {
		if (id === CUSTOM_IMAGE) {
			setCustom(true);
			return;
		}
		setCustom(false);
		if (id === "") {
			setDraft("");
			if (image !== null) onImage(null);
			return;
		}
		const next = `${releases?.repository ?? ""}:${id}`;
		setDraft(next);
		if (next !== image) onImage(next);
	};
	const checkedWords = (() => {
		if (releases === null) return "";
		if (checking) return "Checking…";
		if (releases.error !== undefined) return releases.error;
		const when = releases.checkedAt !== undefined ? ` · checked ${clock(releases.checkedAt)}` : "";
		return `${releases.newest ?? "No release known"}${when}`;
	})();

	useEffect(() => {
		void wire
			.command("computer.runtimes", {})
			.then(setReports)
			.catch(() => setReports([]));
		void wire
			.command("computer.releases", {})
			.then(setReleases)
			.catch(() => setReleases(null));
	}, []);

	const commitImage = () => {
		const trimmed = draft.trim();
		setDraft(trimmed);
		const next = trimmed === "" ? null : trimmed;
		if (next !== image) onImage(next);
	};

	const chosen = runtime ?? "";
	const rows: { id: string; name: string; detail: string; off: boolean; report?: RuntimeReport }[] = [
		{ id: "", name: "Automatic", detail: "First runtime that answers.", off: false },
		...(reports ?? []).map((one) => ({
			id: one.runtime,
			name: RUNTIME_NAMES[one.runtime],
			detail: one.state === "ready" ? (one.rootless ? "Ready, rootless" : "Ready") : STATE_NAMES[one.state],
			off: one.state !== "ready",
			report: one,
		})),
	];

	const [open, setOpen] = useState<"runs" | "image" | null>(null);
	const readyName = reports?.find((one) => one.state === "ready")?.runtime;
	const runsOn =
		chosen === ""
			? `Automatic${readyName !== undefined ? ` · ${RUNTIME_NAMES[readyName]}` : ""}`
			: (RUNTIME_NAMES[chosen as ComputerRuntime] ?? chosen);
	const imageWords =
		pickedRelease === ""
			? releases?.newest !== undefined
				? `Newest · ${releases.newest}`
				: "Newest"
			: pickedRelease === CUSTOM_IMAGE
				? draft.trim() || "Custom image"
				: `${pickedRelease} · pinned`;

	return (
		<><section>
			<h3 className="group-title">Teammate computers</h3>
			<div className="grouped">
				<Fold title="Runs on" value={runsOn} open={open === "runs"} onToggle={() => setOpen((was) => toggled(was, "runs"))}>
					{reports === undefined ? (
						<div className="grouped">
							<p className="group-row text-sm text-ink-3">Looking for a container runtime…</p>
						</div>
					) : (
						<div role="radiogroup" aria-labelledby="setting-computer-runtime" className="grouped">
							{rows.map((row) => {
								const advice = row.report === undefined ? null : runtimeAdvice(row.report);
								const more = row.report !== undefined && (row.report.detail !== undefined || advice !== null);
								const open = more && shown === row.report?.runtime;
								return (
									<Fragment key={row.id}>
										<label className="group-row group-row-choice" data-off={row.off ? "true" : undefined}>
											<input
												type="radio"
												className="radio"
												name="setting-computer-runtime"
												checked={chosen === row.id}
												aria-disabled={row.off ? true : undefined}
												onChange={() => {
													if (row.off) return;
													onRuntime(row.id === "" ? null : row.id);
												}}
											/>
											<span className="group-row-text">
												<span className="group-row-title">{row.name}</span>
												<span className="group-row-detail">{row.detail}</span>
											</span>
											{more && (
												<button
													type="button"
													className="control btn-icon"
													title="Why"
													aria-label={`Why ${row.name} is ${row.detail.toLowerCase()}`}
													aria-expanded={open}
													onClick={(event) => {
														event.preventDefault();
														setShown(open ? null : (row.report?.runtime ?? null));
													}}
												>
													<InfoIcon />
												</button>
											)}
										</label>
										{open && row.report !== undefined && (
											<div className="group-row flex-col items-stretch gap-1.5 pl-10">
												{advice !== null && <span className="text-sm text-ink-2">{advice}</span>}
												{row.report.detail !== undefined && (
													<pre className="refusal-detail selectable">{row.report.detail}</pre>
												)}
												<button
													type="button"
													className="self-start text-sm underline"
													onClick={() => void openLink(RUNTIME_HELP[row.report?.runtime ?? "docker"].docs)}
												>
													{row.name} docs
												</button>
											</div>
										)}
									</Fragment>
								);
							})}
						</div>
					)}
				</Fold>
				<Fold title="Image" value={imageWords} open={open === "image"} onToggle={() => setOpen((was) => toggled(was, "image"))}>
					<Picker field value={pickedRelease} choices={releaseChoices} placeholder="Release" label="Release" onChange={pickRelease} />
					{pickedRelease === CUSTOM_IMAGE && (
						<input
							id="setting-computer-image"
							aria-label="Desktop image"
							className="field font-mono text-sm"
							placeholder={pinnedComputerImage() || "registry/name:tag"}
							autoComplete="off"
							spellCheck={false}
							// biome-ignore lint/a11y/noAutofocus: it is only on when the field appears because Custom image was just picked.
							autoFocus={custom}
							value={draft}
							onChange={(event) => setDraft(event.target.value)}
							onBlur={commitImage}
							onKeyDown={(event) => {
								if (event.key !== "Enter") return;
								event.preventDefault();
								commitImage();
							}}
						/>
					)}
					<p className="hint">Newest follows each release. Pick one to pin it.</p>
				</Fold>
				<div className="nt-fold-row">
					<span className="nt-fold-title">Updates</span>
					<span className="nt-fold-value selectable">{checkedWords}</span>
					<button type="button" className="nt-fold-action" disabled={checking} onClick={() => void checkReleases()}>
						{checking ? "Checking…" : "Check now"}
					</button>
				</div>
			</div>
			<p className="group-hint">
				A teammate&rsquo;s computer is a Linux desktop in a container.{" "}
				<button type="button" className="link-quiet" onClick={() => void openLink(COMPUTER_REPO)}>
					Read more →
				</button>
			</p>
		</section>
		<CookieOfferSettings settings={cookieOffers} onSites={onSites} /></>
	);
}
