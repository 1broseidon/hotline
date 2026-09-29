import { useState } from "react";
import { chordKeys } from "../chords";
import { CloseIcon } from "../icons";
import { setActiveDesk } from "../desks";
import { pairDeskByLink, pairDeskOverSsh } from "../native";
import { Band } from "../ui/Band";
import { onTablistKey } from "../ui/Menu";
import { Refusal } from "../ui/Refusal";
import { Scroll } from "../ui/Scroll";

type Way = "link" | "ssh";

/**
 * Links this window to a desk on a server (BRO-145), as its owner. Two ways,
 * both carrying the same one-time payload the phone's QR does:
 *
 * - a link: run `hotline pair --link` on the server and paste what it prints;
 * - SSH: the window runs `hotline pair --json` on the server through the
 *   person's own `ssh`, so nothing is copied and the trust is the SSH access
 *   they already have.
 *
 * Either way the invitation lasts two minutes and pairs one device.
 */
export function AddDesk({ onClose }: { onClose(): void }) {
	const [way, setWay] = useState<Way>("link");
	const [link, setLink] = useState("");
	const [target, setTarget] = useState("");
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	const ready = way === "link" ? link.trim().startsWith("hotline://pair") : /^[^\s@]+@[^\s@]+$|^[^\s@]+$/.test(target.trim());

	const pair = async () => {
		setBusy(true);
		setRefusal(null);
		try {
			const deskId = way === "link" ? await pairDeskByLink(link.trim()) : await pairDeskOverSsh(target.trim());
			setActiveDesk(deskId);
			onClose();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="pane">
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Add a server</h2>
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					<p className="text-ink-2">
						Link this computer to a Hotline desk running on a server, as its owner. Its teammates, schedules and
						computers keep running there; this window becomes one of the ways you reach them, alongside your phone.
					</p>

					<div className="segmented self-start" role="tablist" aria-label="How to pair" onKeyDown={onTablistKey}>
						<button type="button" role="tab" className="segment" aria-selected={way === "link"} onClick={() => setWay("link")}>
							Paste a link
						</button>
						<button type="button" role="tab" className="segment" aria-selected={way === "ssh"} onClick={() => setWay("ssh")}>
							Pair over SSH
						</button>
					</div>

					{way === "link" ? (
						<section>
							<h3 className="group-title">On the server</h3>
							<p className="selectable rounded-md bg-well px-3 py-2 font-mono text-sm">hotline pair --link</p>
							<p className="group-hint">It prints a link that works once, for two minutes. Paste it here.</p>
							<label className="label mt-4" htmlFor="desk-link">
								Link
							</label>
							<textarea
								id="desk-link"
								className="field h-20 w-full font-mono text-sm"
								placeholder="hotline://pair?p=…"
								value={link}
								spellCheck={false}
								onChange={(event) => setLink(event.target.value)}
							/>
						</section>
					) : (
						<section>
							<h3 className="group-title">Your SSH access</h3>
							<p className="group-hint">
								Hotline runs <code>hotline pair --json</code> on the server with your own <code>ssh</code>, and reads the
								answer back. Use a host you can already reach without a password prompt, or one in your SSH config.
							</p>
							<label className="label mt-4" htmlFor="desk-ssh">
								Server
							</label>
							<input
								id="desk-ssh"
								className="field w-full font-mono text-sm"
								placeholder="you@server.example"
								value={target}
								spellCheck={false}
								onChange={(event) => setTarget(event.target.value)}
								onKeyDown={(event) => {
									if (event.key === "Enter" && ready && !busy) void pair();
								}}
							/>
						</section>
					)}

					<div className="flex justify-end">
						<button type="button" className="control btn btn-primary" disabled={!ready || busy} onClick={() => void pair()}>
							{busy ? "Pairing…" : "Pair"}
						</button>
					</div>
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}
