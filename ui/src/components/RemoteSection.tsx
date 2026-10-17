import { useEffect, useRef, useState } from "react";
import type { DeviceRole, PairedRelay, RemoteStatus, SealedPairing } from "../generated/contract";
import { allDesks, useDesks, wireFor, type Desk } from "../desks";
import { declineRelay, forgetRelay, moveDeskToRelay, pairDeskByLink, writeClipboard } from "../native";
import { Fold } from "../ui/Fold";
import { Picker } from "../ui/Menu";
import { Refusal } from "../ui/Refusal";
import { wire, type RosterEntry } from "../wire";

/** A relay device may only stand in on this desk's relay (ADR 0006). */
const ROLE_LABELS: Record<DeviceRole, string> = { owner: "Owner", companion: "Companion", relay: "Relay only" };

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
	// A tunnel runs on this computer, so it reaches a listener on all host IPs
	// through localhost, and one bound to a single address only through it.
	const target = status.endpoint && (status.host === "all" ? `https://localhost:${new URL(status.endpoint).port}` : status.endpoint);
	const changed = text.trim() !== saved;
	return (
		<Fold title="Public address" value={saved || "None"} action={saved ? "Change" : "Add"} open={open} onToggle={() => setOpen((was) => !was)}>
			<form className="flex items-center gap-2" onSubmit={(e) => { e.preventDefault(); void onSave(text.trim() || null).then(() => setOpen(false)); }}>
				<input className="field min-w-0 flex-1" type="url" inputMode="url" spellCheck={false} placeholder="https://desk.example.com" aria-label="Public address" value={text} disabled={busy} onChange={(e) => setText(e.target.value)} />
				<button type="submit" className="control btn btn-primary shrink-0" disabled={busy || !changed}>Save</button>
				{saved && <button type="button" className="control btn-quiet shrink-0 text-danger" disabled={busy} onClick={() => void onSave(null).then(() => setOpen(false))}>Remove</button>}
			</form>
			<p className="hint">
				{target ? <>Optional, for a tunnel. Point it at <code>{target}</code> (self-signed).</> : "Optional, for a tunnel. Turn Remote access on first."}
			</p>
		</Fold>
	);
}

/**
 * Whether to offer keeping `desk`'s server as a relay only (ADR 0006): its
 * hello on this session said it runs a relay, the person has not declined,
 * and this desktop has no teammates there. The roster is asked for afresh,
 * since a kept one may be from before a teammate was made.
 */
function useRelayOffer(desk: Desk | undefined): boolean {
	const [offer, setOffer] = useState(false);
	const deskId = desk?.id;
	const declined = desk?.relayDeclined === true;
	// A new endpoint is a new connection, and its hello is the one to read.
	const endpoint = desk === undefined ? "" : `${desk.origin} ${desk.token}`;
	useEffect(() => {
		setOffer(false);
		if (deskId === undefined || declined) return;
		const remote = wireFor(deskId);
		remote.connect();
		return remote.subscribe<RosterEntry>({ view: "roster" }, {
			snapshot: (entries) => setOffer(entries.length === 0 && (remote.serves()?.includes("relay") ?? false)),
			event: () => setOffer(false),
		});
	}, [deskId, declined, endpoint]);
	return offer;
}

/**
 * What the shell kept from pairing `deskId`: a relay's invitation (`hotline
 * pair --relay --link`, or any on a relay-only server) is kept as a
 * relay, not a desk (ADR 0006). A new relay becomes this desk's relay when it
 * has none, so pairing one is enough to use it; a relay already chosen is left
 * for the Relay picker to change. `relay` is undefined when it paired a desk.
 * Relays are this computer's, so its own desk is asked whichever is active.
 */
export async function keepPairedRelay(deskId: string): Promise<{ relays: PairedRelay[]; relay: PairedRelay | undefined; status: RemoteStatus | null }> {
	const local = allDesks().find((desk) => desk.kind === "local");
	if (local === undefined) throw new Error("This window has no desk of its own to keep a relay.");
	const core = wireFor(local.id);
	const relays = await core.command("remote.relays", {});
	const relay = relays.find((one) => one.deskId === deskId);
	if (relay === undefined) return { relays, relay, status: null };
	const status = await core.command("remote.status", {});
	return { relays, relay, status: status.relay ? status : await core.command("remote.relay", { deskId }) };
}

/**
 * The servers this desktop is paired with only as a relay, and pairing
 * another from the link `hotline pair --relay --link` prints there.
 */
function RelayServers({ relays, busy, onAdd, onForget }: { relays: PairedRelay[]; busy: boolean; onAdd(link: string): Promise<boolean>; onForget(deskId: string): void }) {
	const [open, setOpen] = useState(false);
	const [link, setLink] = useState("");
	const ready = link.trim().startsWith("hotline://pair");
	return (
		<Fold title="Relay servers" value={relays.length > 0 ? `${relays.length} paired` : "None"} action="Add a relay" open={open} onToggle={() => setOpen((was) => !was)}>
			{relays.length > 0 && <ul className="space-y-1">{relays.map((relay) => <li key={relay.deskId} className="flex items-center gap-2">
				<span className="min-w-0 flex-1 truncate text-sm">{relay.name}</span>
				<button type="button" className="control btn-quiet btn-sm text-danger" disabled={busy} onClick={() => onForget(relay.deskId)}>Forget</button>
			</li>)}</ul>}
			<p className="hint">On the server, run <code>hotline pair --relay --link</code> and paste the link here. It works once, for two minutes.</p>
			<form className="flex items-center gap-2" onSubmit={(e) => { e.preventDefault(); void onAdd(link.trim()).then((added) => { if (added) setLink(""); }); }}>
				<input className="field min-w-0 flex-1 font-mono text-sm" spellCheck={false} placeholder="hotline://pair?p=…" aria-label="Relay link" value={link} disabled={busy} onChange={(e) => setLink(e.target.value)} />
				<button type="submit" className="control btn btn-primary shrink-0" disabled={busy || !ready}>Pair</button>
			</form>
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
	const [relays, setRelays] = useState<PairedRelay[]>([]);
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
		void wire.command("remote.relays", {}).then((paired) => { if (active) setRelays(paired); }).catch((e: unknown) => {
			if (active) setError(String(e));
		});
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
	// A relay is a server this computer pairs with, as a desk or as a relay
	// only; the server carries sealed records it cannot read, and phones
	// still pin this desk.
	const desks = useDesks().filter((desk) => desk.kind === "remote");
	const relayOnly = relays.filter((one) => !desks.some((desk) => desk.id === one.deskId));
	const relay = status?.relay ?? null;
	const relayDesk = desks.find((desk) => desk.id === relay?.deskId);
	const offered = useRelayOffer(relayDesk);
	const relayThrough = (deskId: string | null) => run(async () => {
		setStatus(await wire.command("remote.relay", deskId ? { deskId } : {}));
	});
	const addRelay = async (link: string) => {
		let added = false;
		await run(async () => {
			const kept = await keepPairedRelay(await pairDeskByLink(link));
			setRelays(kept.relays);
			// A desk's link pairs a desk, which the shell keeps as one.
			if (!kept.status) throw new Error("That link paired a desk, not a relay. It is under Desks.");
			setStatus(kept.status);
			added = true;
		});
		return added;
	};
	const forget = (deskId: string) => run(async () => {
		// The core keeps its chosen relay by id, and forgetting the server
		// does not clear it: stop standing in on it first.
		if (relay?.deskId === deskId) setStatus(await wire.command("remote.relay", {}));
		await forgetRelay(deskId);
		setRelays(await wire.command("remote.relays", {}));
	});
	const moveToRelay = (deskId: string) => run(async () => {
		await moveDeskToRelay(deskId);
		setRelays(await wire.command("remote.relays", {}));
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
				{(desks.length > 0 || relayOnly.length > 0 || relay) && <div className="group-row">
					<span className="group-row-text">
						<span className="group-row-title">Relay</span>
						<span className="group-row-detail">{relay?.url ? `Phones also reach this desk through ${relay.name}.` : "Through a server it is paired with, no VPN needed."}</span>
					</span>
					<Picker
						value={relay?.deskId ?? ""}
						choices={[
							{ id: "", name: "None" },
							...(relay && !desks.some((desk) => desk.id === relay.deskId) && !relayOnly.some((one) => one.deskId === relay.deskId) ? [{ id: relay.deskId, name: relay.name }] : []),
							...desks.map((desk) => ({ id: desk.id, name: desk.name })),
							// Labelled by this desktop's grant there, which pairing fixed;
							// what the server runs can change after it and is not kept.
							...relayOnly.map((one) => ({ id: one.deskId, name: one.name, detail: "Relay" })),
						]}
						placeholder="None"
						label="Relay"
						disabled={busy || !status}
						onChange={(id) => void relayThrough(id || null)}
					/>
				</div>}
				<RelayServers relays={relayOnly} busy={busy} onAdd={addRelay} onForget={(deskId) => void forget(deskId)} />
				{status && <PublicAddress key={status.publicUrl ?? ""} status={status} busy={busy} onSave={(url) => run(async () => {
					setStatus(await wire.command("remote.public_url", url ? { url } : {}));
				})} />}
				{status?.enabled && <Fold title="Addresses" value={`${status.endpoints.length} this desk answers on`} action="Show" open={showAddresses} onToggle={() => setShowAddresses((was) => !was)}>
					<ul className="space-y-1 font-mono text-sm text-ink-2">{status.endpoints.map((endpoint) => <li key={endpoint} className="selectable break-all">{endpoint}</li>)}</ul>
				</Fold>}
			</div>
			{relay?.error && status?.enabled && <p className="group-hint">{relay.name}: {relay.error}</p>}
			{offered && relayDesk && <div className="nt-card mt-3" role="group" aria-label="Relay only">
				<p className="text-sm text-ink-2">Use {relayDesk.name} only as your relay? Your desktop will no longer be able to run or read anything on it.</p>
				<div className="flex flex-wrap gap-2">
					<button type="button" className="control btn-primary" disabled={busy} onClick={() => void moveToRelay(relayDesk.id)}>Use only as relay</button>
					<button type="button" className="control btn" disabled={busy} onClick={() => void run(() => declineRelay(relayDesk.id))}>Keep as desk</button>
				</div>
			</div>}
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
				<div className="group-row-text"><span className="group-row-title">{device.name}</span><span className="group-row-detail">{ROLE_LABELS[device.role]} · {device.publicKey ? `Paired ${new Date(Number(device.pairedAt)).toLocaleDateString()}` : "Needs re-pair · Scan a new QR"}</span></div>
				<button type="button" className="control btn-quiet text-danger" disabled={busy} onClick={() => void run(async () => { setStatus(await wire.command("remote.revoke", { deviceId: device.id })); setPairing(null); })}>Revoke</button>
			</div>)}</div>
		</section>}
		{(error || status?.error) && <Refusal message={error || status?.error || ""} />}
	</>;
}
