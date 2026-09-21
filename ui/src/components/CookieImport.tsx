import { useEffect, useMemo, useState } from "react";
import type { CookieImport as CookieImportRecord, CookieSite, HostBrowser } from "../generated/contract";
import { CloseIcon } from "../icons";
import { wire } from "../wire";

const NESTED = "group-row pl-7";

function reason(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

/**
 * The operator's cookie picker: choose a browser on this machine, tick the
 * sites to bring over, and hand those cookies to the teammate's computer so
 * its browser starts signed in to them.
 *
 * This is an operator action, and it looks like one: the person picks the
 * browser, the profile and the exact sites. The window only ever sees site
 * names and counts — a cookie value never leaves the desk for the window, and
 * the agent has no way to start any of this. The three commands are desk-seat
 * only.
 */
export function CookieImport({
	personaId,
	running,
	onClose,
}: {
	personaId: string;
	running: boolean;
	onClose(): void;
}) {
	const [browsers, setBrowsers] = useState<HostBrowser[] | null>(null);
	const [browserId, setBrowserId] = useState("");
	const [profileId, setProfileId] = useState("");
	const [sites, setSites] = useState<CookieSite[] | null>(null);
	const [chosen, setChosen] = useState<Set<string>>(new Set());
	const [filter, setFilter] = useState("");
	const [busy, setBusy] = useState(false);
	const [note, setNote] = useState<string | null>(null);
	const [done, setDone] = useState<CookieSite[] | null>(null);

	// The browsers on the host, asked for once when the picker opens.
	useEffect(() => {
		let gone = false;
		wire
			.command("computer.browsers.list", {})
			.then((list) => {
				if (gone) return;
				setBrowsers(list);
				const first = list[0];
				if (first) {
					setBrowserId(first.id);
					setProfileId(first.profiles[0]?.id ?? "");
				}
			})
			.catch((error) => !gone && setNote(reason(error)));
		return () => {
			gone = true;
		};
	}, []);

	const browser = browsers?.find((entry) => entry.id === browserId);

	// The sites in the chosen profile, re-read whenever the choice changes.
	// Names and counts only; no value is ever requested.
	useEffect(() => {
		if (browserId === "" || profileId === "") {
			setSites(null);
			return;
		}
		let gone = false;
		setSites(null);
		setChosen(new Set());
		setDone(null);
		setNote(null);
		wire
			.command("computer.cookies.preview", { browserId, profileId })
			.then((found) => !gone && setSites(found))
			.catch((error) => !gone && setNote(reason(error)));
		return () => {
			gone = true;
		};
	}, [browserId, profileId]);

	const shown = useMemo(() => {
		const needle = filter.trim().toLowerCase();
		return (sites ?? []).filter((site) => site.domain.includes(needle));
	}, [sites, filter]);

	const toggle = (domain: string) =>
		setChosen((prev) => {
			const next = new Set(prev);
			if (next.has(domain)) {
				next.delete(domain);
			} else {
				next.add(domain);
			}
			return next;
		});

	const allShownChosen = shown.length > 0 && shown.every((site) => chosen.has(site.domain));
	const toggleAllShown = () =>
		setChosen((prev) => {
			const next = new Set(prev);
			if (allShownChosen) {
				for (const site of shown) next.delete(site.domain);
			} else {
				for (const site of shown) next.add(site.domain);
			}
			return next;
		});

	const runImport = async () => {
		setBusy(true);
		setNote(null);
		try {
			const imported = await wire.command("computer.cookies.import", {
				personaId,
				browserId,
				profileId,
				domains: [...chosen],
			});
			setDone(imported);
		} catch (error) {
			setNote(reason(error));
		} finally {
			setBusy(false);
		}
	};

	if (done !== null) {
		const cookies = done.reduce((sum, site) => sum + site.cookies, 0);
		return (
			<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
				<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
					{done.length === 0
						? "Nothing was imported."
						: `${browser?.name ?? "The browser"}'s cookies for ${done.length} ${done.length === 1 ? "site" : "sites"} (${cookies} in all) are in the computer's browser now.`}
				</span>
				<div className="flex justify-end">
					<button type="button" className="control btn" onClick={onClose}>
						Done
					</button>
				</div>
			</div>
		);
	}

	return (
		<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
			<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
				Pick the sites to bring over. The teammate's browser will be signed in to
				them. The window never sees a cookie's value, and the agent cannot do this
				itself.
			</span>

			{browsers !== null && browsers.length === 0 && (
				<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
					No browser with cookies was found on this machine.
				</span>
			)}

			{browsers !== null && browsers.length > 0 && (
				<>
					<div className="flex gap-2">
						<select
							className="field flex-1 text-sm"
							aria-label="Browser"
							value={browserId}
							onChange={(event) => {
								const next = browsers.find((entry) => entry.id === event.target.value);
								setBrowserId(event.target.value);
								setProfileId(next?.profiles[0]?.id ?? "");
							}}
						>
							{browsers.map((entry) => (
								<option key={entry.id} value={entry.id}>
									{entry.name}
								</option>
							))}
						</select>
						{browser && browser.profiles.length > 1 && (
							<select
								className="field flex-1 text-sm"
								aria-label="Profile"
								value={profileId}
								onChange={(event) => setProfileId(event.target.value)}
							>
								{browser.profiles.map((profile) => (
									<option key={profile.id} value={profile.id}>
										{profile.name}
									</option>
								))}
							</select>
						)}
					</div>

					{sites === null && note === null && (
						<span className="group-row-detail">Reading {browser?.name ?? "the browser"}…</span>
					)}

					{sites !== null && (
						<>
							<div className="flex items-center gap-2">
								<input
									className="field flex-1 text-sm"
									placeholder="Filter sites"
									autoComplete="off"
									spellCheck={false}
									value={filter}
									onChange={(event) => setFilter(event.target.value)}
								/>
								<button
									type="button"
									className="control btn-quiet btn-sm"
									disabled={shown.length === 0}
									onClick={toggleAllShown}
								>
									{allShownChosen ? "None" : "All"}
								</button>
							</div>

							<div className="flex max-h-64 flex-col overflow-y-auto">
								{shown.length === 0 ? (
									<span className="group-row-detail py-1">No sites match.</span>
								) : (
									shown.map((site) => (
										<label
											key={site.domain}
											className="flex cursor-pointer items-center gap-2 py-1 text-sm"
										>
											<input
												type="checkbox"
												className="check"
												checked={chosen.has(site.domain)}
												onChange={() => toggle(site.domain)}
											/>
											<span className="min-w-0 flex-1 truncate font-mono">{site.domain}</span>
											<span className="group-row-detail font-mono">{site.cookies}</span>
										</label>
									))
								)}
							</div>
						</>
					)}
				</>
			)}

			{note !== null && (
				<span className="group-row-detail text-danger" style={{ whiteSpace: "normal" }}>
					{note}
				</span>
			)}

			{sites !== null && !running && chosen.size > 0 && (
				<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
					The computer will start to receive them.
				</span>
			)}

			<div className="flex justify-end gap-2">
				<button type="button" className="control btn-quiet" onClick={onClose} disabled={busy}>
					Cancel
				</button>
				<button
					type="button"
					className="control btn"
					disabled={busy || chosen.size === 0}
					onClick={() => void runImport()}
				>
					{busy
						? "Bringing over…"
						: chosen.size === 0
							? "Bring over"
							: `Bring over ${chosen.size} ${chosen.size === 1 ? "site" : "sites"}`}
				</button>
			</div>
		</div>
	);
}

function when(ms: number): string {
	return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/**
 * What has been brought over to this teammate's computer: each browser and
 * profile, when, and the sites with their counts — and the way to take it
 * back, one site at a time or the whole browser's worth. Taking back names
 * the exact sites to the computer, whose browser drops those cookies at
 * once; a stopped computer is started to do it, as it is to receive them.
 * Names and counts only, as everywhere on the desk.
 */
export function CookieImports({
	personaId,
	disabled,
	refresh,
}: {
	personaId: string;
	disabled: boolean;
	refresh: number;
}) {
	const [imports, setImports] = useState<CookieImportRecord[] | null>(null);
	const [busy, setBusy] = useState<string | null>(null);
	const [note, setNote] = useState<string | null>(null);

	// The record is re-read whenever the picker closes, so a fresh import
	// shows up without a reload.
	useEffect(() => {
		let gone = false;
		wire
			.command("computer.cookies.list", { personaId })
			.then((list) => !gone && setImports(list))
			.catch((error) => !gone && setNote(reason(error)));
		return () => {
			gone = true;
		};
	}, [personaId, refresh]);

	const forget = async (entry: CookieImportRecord, domain?: string) => {
		setBusy(`${entry.browserId}/${entry.profileId}/${domain ?? "*"}`);
		setNote(null);
		try {
			const left = await wire.command("computer.cookies.forget", {
				personaId,
				browserId: entry.browserId,
				profileId: entry.profileId,
				...(domain === undefined ? {} : { domain }),
			});
			setImports(left);
		} catch (error) {
			setNote(reason(error));
		} finally {
			setBusy(null);
		}
	};

	const shown = imports ?? [];
	if (shown.length === 0 && note === null) return null;

	return (
		<div className={`${NESTED} flex-col items-stretch gap-2 py-3`}>
			{shown.map((entry) => {
				const key = `${entry.browserId}/${entry.profileId}`;
				return (
					<div key={key} className="flex flex-col gap-1">
						<div className="flex items-center gap-2 text-sm">
							<span className="min-w-0 flex-1 truncate">
								<span className="group-row-title">{entry.browserName}</span>
								<span className="group-row-detail ml-2">
									{entry.profileName} · {when(entry.importedAt)}
								</span>
							</span>
							<button
								type="button"
								className="control btn-quiet btn-sm"
								disabled={disabled || busy !== null}
								onClick={() => void forget(entry)}
							>
								{busy === `${key}/*` ? "Removing…" : "Remove all"}
							</button>
						</div>
						{entry.sites.map((site) => (
							<div key={site.domain} className="flex items-center gap-2 py-0.5 pl-2 text-sm">
								<span className="min-w-0 flex-1 truncate font-mono">{site.domain}</span>
								<span className="group-row-detail font-mono">{site.cookies}</span>
								<button
									type="button"
									className="control btn-icon btn-quiet h-6 w-6 text-ink-3"
									title={`Remove ${site.domain}`}
									aria-label={`Remove ${site.domain}`}
									disabled={disabled || busy !== null}
									onClick={() => void forget(entry, site.domain)}
								>
									<CloseIcon />
								</button>
							</div>
						))}
					</div>
				);
			})}
			{note !== null && (
				<span className="group-row-detail text-danger" style={{ whiteSpace: "normal" }}>
					{note}
				</span>
			)}
		</div>
	);
}
