import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import type { RemotePairing, RemoteStatus } from "../generated/contract";
import { writeClipboard } from "../native";
import { Refusal } from "../ui/Refusal";

/** What the phone accepts: host:port, with IPv6 in brackets. */
function manualAddress(address: string, port: number): string {
	return `${address.includes(":") ? `[${address}]` : address}:${port}`;
}

export function RemoteSection() {
	const [status, setStatus] = useState<RemoteStatus | null>(null);
	const [pairing, setPairing] = useState<RemotePairing | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const [clock, setClock] = useState(Date.now());
	const [copied, setCopied] = useState(false);
	const revision = useRef(0);
	const pending = useRef(false);
	useEffect(() => {
		let active = true;
		const refresh = () => {
			if (pending.current) return;
			const started = revision.current;
			void invoke<RemoteStatus>("remote_status").then((next) => {
				if (active && started === revision.current) setStatus(next);
			}).catch((e: unknown) => {
				if (active && started === revision.current) setError(String(e));
			});
		};
		refresh();
		const timer = setInterval(() => { setClock(Date.now()); refresh(); }, 2000);
		return () => { active = false; clearInterval(timer); };
	}, []);
	const run = async (action: () => Promise<void>) => {
		if (pending.current) return;
		pending.current = true;
		revision.current++;
		setBusy(true);
		setError(null);
		try { await action(); } catch (e) { setError(String(e)); }
		finally {
			revision.current++;
			pending.current = false;
			setBusy(false);
		}
	};
	const configure = (enabled: boolean, host = status?.host ?? "all") => run(async () => {
		setPairing(null);
		setCopied(false);
		setStatus(await invoke<RemoteStatus>("remote_configure", { enabled, host }));
	});
	const expired = pairing !== null && Number(pairing.invitation.expiresAt) <= clock;
	const linked = pairing !== null && status?.devices.some((device) => Number(device.pairedAt) >= Number(pairing.invitation.expiresAt) - 120_000);
	const host = status?.host ?? "all";
	return <>
		<section aria-label="Remote access">
			<h3 className="group-title">Connection</h3>
			<div className="grouped">
				<label className="group-row group-row-choice">
					<span className="group-row-text">
						<span className="group-row-title">Remote access</span>
						<span className="group-row-detail">{status?.enabled ? "On" : "Off"} · Connect paired phones over your network or VPN.</span>
					</span>
					<input type="checkbox" role="switch" className="switch" aria-label="Remote access" checked={status?.enabled ?? false} disabled={busy || !status} onChange={(e) => void configure(e.target.checked)} />
				</label>
				<div className="group-row">
					<label className="group-row-text" htmlFor="remote-address">
						<span className="group-row-title">Listen on</span>
						<span className="group-row-detail">All host IPs by default. Choose one to limit access.</span>
					</label>
					<select id="remote-address" className="field max-w-56" value={host} disabled={busy || !status} onChange={(e) => void configure(status?.enabled ?? false, e.target.value)}>
						<option value="all">All host IPs</option>
						{host !== "all" && !status?.addresses.includes(host) && <option value={host} disabled>{host} (unavailable)</option>}
						{status?.addresses.map((address) => <option key={address} value={address}>{address}</option>)}
					</select>
				</div>
			</div>
			<p className="group-hint">Turning Remote off disconnects every phone.</p>
			{status?.enabled && <details className="group-hint">
				<summary className="cursor-pointer">Listening addresses</summary>
				<ul className="mt-2 space-y-1">{status.endpoints.map((endpoint) => <li key={endpoint} className="break-all">{endpoint}</li>)}</ul>
			</details>}
		</section>
		{status?.enabled && <section aria-label="Mobile pairing">
			<h3 className="group-title">Link a phone</h3>
			<div className="grouped p-5">
				<p className="text-sm text-ink-2">Scan the code in Toad on your phone, or type the address and the six digits.</p>
				{pairing && !expired && !linked && <div className="mt-5 flex flex-col items-center gap-3">
					<img className="h-72 w-72 max-w-full rounded-lg bg-white" src={`data:image/svg+xml,${encodeURIComponent(pairing.qrSvg)}`} alt="Scan with Toad to pair this desktop" />
					<dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm" aria-label="Type this into your phone">
						<dt className="text-ink-3">Address</dt>
						<dd className="font-mono break-all">{manualAddress(pairing.manual.address, pairing.manual.port)}</dd>
						<dt className="text-ink-3">Code</dt>
						<dd className="font-mono text-xl tracking-[0.3em]">{pairing.manual.code}</dd>
					</dl>
					<p className="text-sm text-ink-3">Expires in {Math.max(0, Math.ceil((Number(pairing.invitation.expiresAt) - clock) / 1000))} seconds.</p>
				</div>}
				{linked && <p role="status" className="mt-4 text-sm text-accent">Phone linked.</p>}
				{expired && !linked && <p role="status" className="mt-4 text-sm text-ink-2">This code expired. Create a new one to pair.</p>}
				<div className="mt-5 flex flex-wrap gap-2">
					<button type="button" className="control btn-primary" disabled={busy} onClick={() => void run(async () => { setPairing(await invoke<RemotePairing>("remote_pairing")); setClock(Date.now()); setCopied(false); })}>{pairing ? "New pairing code" : "Show pairing code"}</button>
					{pairing && !expired && !linked && <button type="button" className="control btn" disabled={busy} onClick={() => void run(async () => { await writeClipboard(manualAddress(pairing.manual.address, pairing.manual.port)); setCopied(true); })}>{copied ? "Copied" : "Copy address"}</button>}
				</div>
			</div>
		</section>}
		{status && status.devices.length > 0 && <section aria-label="Paired phones">
			<h3 className="group-title">Paired phones</h3>
			<div className="grouped">{status.devices.map((device) => <div key={device.id} className="group-row">
				<div className="group-row-text"><span className="group-row-title">{device.name}</span><span className="group-row-detail">Paired {new Date(Number(device.pairedAt)).toLocaleDateString()}</span></div>
				<button type="button" className="control btn" disabled={busy} onClick={() => void run(async () => { setStatus(await invoke<RemoteStatus>("remote_revoke", { deviceId: device.id })); setPairing(null); })}>Revoke access</button>
			</div>)}</div>
		</section>}
		{(error || status?.error) && <Refusal message={error || status?.error || ""} />}
	</>;
}
