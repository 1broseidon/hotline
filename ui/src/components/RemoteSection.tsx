import { useEffect, useRef, useState } from "react";
import type { RemoteStatus, SealedPairing } from "../generated/contract";
import { useDesks } from "../desks";
import { writeClipboard } from "../native";
import { Fold } from "../ui/Fold";
import { Picker } from "../ui/Menu";
import { Refusal } from "../ui/Refusal";
import { wire } from "../wire";

function cancelPairing(id: string) {
	return wire.command("remote.pairing", { id, cancel: true }).catch(() => {});
}

/**
 * Optional, and quiet until someone has one: the https address a tunnel, a
 * reverse proxy or a forwarded port gives this desk. Phones learn it and try
 * it after this computer's own addresses, so pairing still happens on the
 * network the phone is standing in.
 */
function PublicAddress({ status, busy, onSave }: { status: RemoteStatus; busy: boolean; onSave: (url: string | null) => Promise<void> }) {
	const saved = status.publicUrl ?? "";
	const [text, setText] = useState(saved);
	const [open, setOpen] = useState(false);
	const port = status.endpoint ? new URL(status.endpoint).port : null;
	const changed = text.trim() !== saved;
	return (
		<Fold title="Public address" value={saved || "None"} action={saved ? "Change" : "Add"} open={open} onToggle={() => setOpen((was) => !was)}>
			<form className="flex items-center gap-2" onSubmit={(e) => { e.preventDefault(); void onSave(text.trim() || null).then(() => setOpen(false)); }}>
				<input className="field min-w-0 flex-1" type="url" inputMode="url" spellCheck={false} placeholder="https://desk.example.com" aria-label="Public address" value={text} disabled={busy} onChange={(e) => setText(e.target.value)} />
				<button type="submit" className="control btn btn-primary shrink-0" disabled={busy || !changed}>Save</button>
				{saved && <button type="button" className="control btn-quiet shrink-0 text-danger" disabled={busy} onClick={() => void onSave(null).then(() => setOpen(false))}>Remove</button>}
			</form>
			<p className="hint">
				{port ? <>Optional, for a tunnel. Point it at <code>https://localhost:{port}</code> (self-signed).</> : "Optional, for a tunnel. Turn Remote access on first."}
			</p>
		</Fold>
	);
}

export function RemoteSection() {
	const [status, setStatus] = useState<RemoteStatus | null>(null);
	const [pairing, setPairing] = useState<SealedPairing | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const [clock, setClock] = useState(Date.now());
	const [copied, setCopied] = useState(false);
	const [linked, setLinked] = useState(false);
	const [showAddresses, setShowAddresses] = useState(false);
	const revision = useRef(0);
	const pending = useRef(false);
	const mounted = useRef(false);
	useEffect(() => {
		mounted.current = true;
		let active = true;
		let refreshing = false;
		const refresh = () => {
			if (pending.current || refreshing) return;
			refreshing = true;
			const started = revision.current;
			void wire.command("remote.status", {}).then((next) => {
				if (active && started === revision.current) setStatus(next);
			}).catch((e: unknown) => {
				if (active && started === revision.current) setError(String(e));
			}).finally(() => { refreshing = false; });
		};
		refresh();
		const timer = setInterval(() => { setClock(Date.now()); refresh(); }, 2000);
		return () => { active = false; mounted.current = false; clearInterval(timer); };
	}, []);
	useEffect(() => {
		setLinked(false);
		if (!pairing) return;
		let active = true;
		let polling = false;
		const poll = async () => {
			if (Number(pairing.expiresAt) <= Date.now()) { clearInterval(timer); return; }
			if (polling || pending.current) return;
			polling = true;
			const started = revision.current;
			try {
				const result = await wire.command("remote.pairing", { id: pairing.id, cancel: false });
				if (active && started === revision.current && result && "pairedAt" in result) {
					clearInterval(timer);
					setLinked(true);
				}
			} catch (e) {
				if (active && started === revision.current) setError(String(e));
			} finally { polling = false; }
		};
		const timer = setInterval(() => void poll(), 2000);
		return () => {
			active = false;
			clearInterval(timer);
			void cancelPairing(pairing.id);
		};
	}, [pairing]);
	const run = async (action: () => Promise<void>) => {
		if (pending.current) return;
		pending.current = true;
		revision.current++;
		setBusy(true);
		setError(null);
		try { await action(); } catch (e) { if (mounted.current) setError(String(e)); }
		finally {
			revision.current++;
			pending.current = false;
			if (mounted.current) setBusy(false);
		}
	};
	const configure = (enabled: boolean, host = status?.host ?? "all") => run(async () => {
		setPairing(null);
		setCopied(false);
		setStatus(await wire.command("remote.configure", { enabled, host }));
	});
	const expiresAt = pairing ? Number(pairing.expiresAt) : 0;
	const expired = pairing !== null && expiresAt <= clock;
	const beginPairing = () => run(async () => {
		// A replacement invalidates the displayed invitation, even while its reply is pending.
		setPairing(null);
		const next = await wire.command("remote.pairing", { cancel: false });
		if (!next || !("qrSvg" in next)) throw new Error("The desk did not return a pairing invitation.");
		if (!mounted.current) {
			await cancelPairing(next.id);
			return;
		}
		setPairing(next);
		setClock(Date.now());
		setCopied(false);
	});
	const host = status?.host ?? "all";
	// A relay is a desk this computer pairs with; the server carries sealed
	// records it cannot read, and phones still pin this desk.
	const relays = useDesks().filter((desk) => desk.kind === "remote");
	const relay = status?.relay ?? null;
	const relayThrough = (deskId: string | null) => run(async () => {
		setStatus(await wire.command("remote.relay", deskId ? { deskId } : {}));
	});
	return <>
		<section aria-label="Remote access">
			<h3 className="group-title">Connection</h3>
			<div className="grouped">
				<label className="group-row group-row-choice">
					<span className="group-row-text">
						<span className="group-row-title">Remote access</span>
						<span className="group-row-detail">{status?.enabled ? "On · Linked phones can reach this desk." : "Off"}</span>
					</span>
					<input type="checkbox" role="switch" className="switch" aria-label="Remote access" checked={status?.enabled ?? false} disabled={busy || !status} onChange={(e) => void configure(e.target.checked)} />
				</label>
				<div className="group-row">
					<span className="group-row-text">
						<span className="group-row-title">Listen on</span>
						<span className="group-row-detail">Which network phones use.</span>
					</span>
					<Picker
						value={host}
						choices={[
							{ id: "all", name: "All host IPs" },
							...(host !== "all" && !status?.addresses.includes(host) ? [{ id: host, name: `${host} (unavailable)` }] : []),
							...(status?.addresses ?? []).map((address) => ({ id: address, name: address })),
						]}
						placeholder="All host IPs"
						label="Listen on"
						disabled={busy || !status}
						onChange={(id) => void configure(status?.enabled ?? false, id)}
					/>
				</div>
				{(relays.length > 0 || relay) && <div className="group-row">
					<span className="group-row-text">
						<span className="group-row-title">Relay</span>
						<span className="group-row-detail">{relay?.url ? `Phones also reach this desk through ${relay.name}.` : "Through a server it is paired with, no VPN needed."}</span>
					</span>
					<Picker
						value={relay?.deskId ?? ""}
						choices={[
							{ id: "", name: "None" },
							...(relay && !relays.some((desk) => desk.id === relay.deskId) ? [{ id: relay.deskId, name: relay.name }] : []),
							...relays.map((desk) => ({ id: desk.id, name: desk.name })),
						]}
						placeholder="None"
						label="Relay"
						disabled={busy || !status}
						onChange={(id) => void relayThrough(id || null)}
					/>
				</div>}
				{status && <PublicAddress key={status.publicUrl ?? ""} status={status} busy={busy} onSave={(url) => run(async () => {
					setStatus(await wire.command("remote.public_url", url ? { url } : {}));
				})} />}
				{status?.enabled && <Fold title="Addresses" value={`${status.endpoints.length} this desk answers on`} action="Show" open={showAddresses} onToggle={() => setShowAddresses((was) => !was)}>
					<ul className="space-y-1 font-mono text-sm text-ink-2">{status.endpoints.map((endpoint) => <li key={endpoint} className="selectable break-all">{endpoint}</li>)}</ul>
				</Fold>}
			</div>
			{relay?.error && status?.enabled && <p className="group-hint">{relay.name}: {relay.error}</p>}
			<p className="group-hint">Turning Remote off disconnects every phone.</p>
		</section>
		{status?.enabled && <section aria-label="Mobile pairing">
			<h3 className="group-title">Link a phone</h3>
			<div className="nt-card">
				<p className="text-sm text-ink-2">Scan the QR in Hotline on your phone, or open the pairing link there.</p>
				{pairing && !expired && !linked && <div className="flex flex-col items-center gap-3">
					<img className="h-72 w-72 max-w-full rounded-lg bg-white" src={`data:image/svg+xml,${encodeURIComponent(pairing.qrSvg)}`} alt="Scan with Hotline to pair this desktop" />
					<p className="text-sm text-ink-3">Expires in {Math.max(0, Math.ceil((expiresAt - clock) / 1000))} seconds.</p>
				</div>}
				{linked && <p role="status" className="text-sm text-accent-ink">Phone linked.</p>}
				{expired && !linked && <p role="status" className="text-sm text-ink-2">This QR expired. Link your phone again.</p>}
				<div className="flex flex-wrap gap-2">
					<button type="button" className="control btn-primary" disabled={busy} onClick={() => void beginPairing()}>Link a phone</button>
					{pairing && !expired && !linked && <button type="button" className="control btn" disabled={busy} onClick={() => void run(async () => { await writeClipboard(pairing.link); setCopied(true); })}>{copied ? "Copied" : "Copy link"}</button>}
				</div>
			</div>
		</section>}
		{status && status.devices.length > 0 && <section aria-label="Paired phones">
			<h3 className="group-title">Paired phones</h3>
			<div className="grouped">{status.devices.map((device) => <div key={device.id} className="group-row">
				<div className="group-row-text"><span className="group-row-title">{device.name}</span><span className="group-row-detail">{device.role === "companion" ? "Companion" : "Owner"} · {device.publicKey ? `Paired ${new Date(Number(device.pairedAt)).toLocaleDateString()}` : "Needs re-pair · Scan a new QR"}</span></div>
				<button type="button" className="control btn-quiet text-danger" disabled={busy} onClick={() => void run(async () => { setStatus(await wire.command("remote.revoke", { deviceId: device.id })); setPairing(null); })}>Revoke</button>
			</div>)}</div>
		</section>}
		{(error || status?.error) && <Refusal message={error || status?.error || ""} />}
	</>;
}
