import { useEffect, useState } from "react";
import type { CookieOfferBlock, CookieOfferSettings as OfferSettings, CookieOfferSites } from "../../generated/contract";
import { ChevronRightIcon } from "../../icons";
import { BackKey, Band } from "../../ui/Band";
import { Scroll } from "../../ui/Scroll";
import { wire } from "../../wire";

/** Opening Computer reads no browser data. Only the Sites page discovers it. */
export function CookieOfferSettings({ settings, onSites }: { settings: OfferSettings; onSites(): void }) {
	const [error, setError] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const toggle = async (enabled: boolean) => {
		setBusy(true);
		setError(null);
		try {
			await wire.command("settings.update", { patch: { cookieOffers: { ...settings, enabled } } });
		} catch (error) {
			setError(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};
	const words = settings.sites === undefined ? "All sites" : settings.sites.length === 1 ? "1 site" : `${settings.sites.length} sites`;
	return (
		<section>
			<h3 className="group-title">Browser sign-in offers</h3>
			<div className="grouped">
				<label className="group-row">
					<span className="group-row-text">Offer your browser cookies</span>
					<input type="checkbox" role="switch" className="switch" aria-checked={settings.enabled} checked={settings.enabled} disabled={busy}
						onChange={(event) => void toggle(event.target.checked)} />
				</label>
				<button type="button" className="group-row w-full text-left" onClick={onSites}>
					<span className="group-row-text">Sites: {words}</span>
					<ChevronRightIcon />
				</button>
			</div>
			<p className="group-hint">Every lend asks first. Turning offers off stops automatic browser discovery.</p>
			{error !== null && <p role="alert" className="text-sm text-danger">{error}</p>}
		</section>
	);
}

export function CookieSitesPage({ settings, onBack }: { settings: OfferSettings; onBack(): void }) {
	const [inventory, setInventory] = useState<CookieOfferSites | null>(null);
	const [blocked, setBlocked] = useState<CookieOfferBlock[] | null>(null);
	const [selected, setSelected] = useState<Set<string> | null>(settings.sites === undefined ? null : new Set(settings.sites));
	const [query, setQuery] = useState("");
	const [error, setError] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	useEffect(() => {
		let live = true;
		void Promise.all([
			wire.command("computer.cookies.sites", {}),
			wire.command("computer.cookies.blocked", {}),
		]).then(([sites, blocks]) => {
			if (!live) return;
			setInventory(sites);
			setBlocked(blocks);
		}).catch((error: unknown) => {
			if (live) setError(error instanceof Error ? error.message : String(error));
		});
		return () => { live = false; };
	}, []);

	// A saved choice stays visible even when its browser or profile is gone.
	const sites = [...(inventory?.sites ?? [])];
	for (const site of settings.sites ?? []) {
		if (!sites.some((row) => row.site === site)) sites.push({ site, sources: [] });
	}
	sites.sort((a, b) => a.site.localeCompare(b.site));
	const all = selected === null || (sites.length > 0 && sites.every((row) => selected.has(row.site)));
	const needle = query.trim().toLowerCase();
	const visible = sites.filter((row) => `${row.site} ${row.sources.map((source) => `${source.browserName} ${source.profileName}`).join(" ")}`.toLowerCase().includes(needle));
	const run = async (work: () => Promise<void>) => {
		if (busy) return;
		setBusy(true);
		setError(null);
		try { await work(); }
		catch (error) { setError(error instanceof Error ? error.message : String(error)); }
		finally { setBusy(false); }
	};
	const save = () => run(async () => {
		await wire.command("settings.update", { patch: { cookieOffers: {
			enabled: settings.enabled,
			...(all ? {} : { sites: [...(selected ?? [])].sort() }),
		} } });
		onBack();
	});
	const undo = (block: CookieOfferBlock) => run(async () => {
		await wire.command("computer.cookies.reset", { personaId: block.teammate, site: block.site });
		setBlocked(await wire.command("computer.cookies.blocked", {}));
	});
	return (
		<div className="pane">
			<Band>
				<BackKey onBack={onBack} />
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Cookie offer sites</h2>
				<button type="button" className="control btn" disabled={busy || inventory === null} onClick={() => void save()}>Save</button>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					<p className="group-hint">Choose where to offer your browser session. Every lend asks first. Account-wide sites such as google.com, apple.com and microsoft.com lend the whole account.</p>
					{error !== null && <p role="alert" className="text-sm text-danger">{error}</p>}
					{inventory === null ? <p className="text-sm text-ink-3">{error === null ? "Reading browser sites…" : "Browser sites could not be loaded. Go back and reopen Sites to retry."}</p> : <section>
						<h3 className="group-title">Sites</h3>
						<input className="field mb-3" aria-label="Search sites" placeholder="Search sites or browsers" value={query} onChange={(event) => setQuery(event.target.value)} />
						<div className="grouped">
							<label className="group-row">
								<span className="group-row-text">All sites</span>
								<input type="checkbox" className="check" checked={all} disabled={busy} onChange={(event) => setSelected(event.target.checked ? null : new Set())} />
							</label>
							{visible.map((row) => <label className="group-row" key={row.site}>
								<span className="group-row-text">
									<span className="group-row-title">{row.site}</span>
									<span className="group-row-detail">{row.sources.length === 0 ? "Not found in browsers on this desk" : row.sources.map((source) => `${source.browserName} · ${source.profileName}`).join("; ")}</span>
								</span>
								<input type="checkbox" className="check" aria-label={row.site} checked={selected === null || selected.has(row.site)} disabled={busy} onChange={(event) => {
									const next = new Set(selected ?? sites.map((one) => one.site));
									if (event.target.checked) next.add(row.site); else next.delete(row.site);
									setSelected(next);
								}} />
							</label>)}
						</div>
						{visible.length === 0 && <p className="group-hint">{sites.length === 0 ? "No browser cookie sites found." : "No matching sites."}</p>}
						{inventory.unavailable.map((message) => <p className="group-hint" key={message}>{message}</p>)}
					</section>}
					{blocked !== null && <section>
						<h3 className="group-title">Never for this site</h3>
						<p className="group-hint">These sites stay hidden for the named teammate until you undo the block.</p>
						{blocked.length === 0 && <p className="text-sm text-ink-3">No sites hidden.</p>}
						<div className="grouped">
							{blocked.filter((block) => `${block.name} ${block.site}`.toLowerCase().includes(needle)).map((block) => <div className="group-row" key={`${block.teammate}/${block.site}`}>
								<span className="group-row-text">{block.name} · {block.site}</span>
								<button type="button" className="control btn" disabled={busy} onClick={() => void undo(block)}>Undo</button>
							</div>)}
						</div>
					</section>}
				</div>
			</Scroll>
		</div>
	);
}
