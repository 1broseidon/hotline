import { useState } from "react";
import { forgetDesk } from "../native";
import { useDesks, type Desk } from "../desks";
import { linkDesk, linkWords, unlinkDesk, useDeskLinks } from "../deskLinks";
import type { DeskLink } from "../generated/contract";
import { PlusIcon } from "../icons";
import { Refusal } from "../ui/Refusal";

/**
 * The desks this window reaches (BRO-145): this computer's, and the servers
 * it is paired with as owner. Forgetting a server stops reaching it from
 * here; the server still lists this computer until it is revoked there.
 *
 * Linking a server (ADR 0003) is a separate, deliberate step: it lets this
 * computer's teammates and the server's see and message each other, each
 * under its own grants. Either side can end it, and ending it ends it on
 * both.
 */
export function DesksSection({ onAddDesk }: { onAddDesk(): void }) {
	const desks = useDesks();
	const links = useDeskLinks();
	const [refusal, setRefusal] = useState<string | null>(null);
	const [confirming, setConfirming] = useState<{ deskId: string; action: "forget" | "unlink" } | null>(null);
	const [linking, setLinking] = useState<string | null>(null);
	const local = desks.some((desk) => desk.kind === "local");

	const act = async (deskId: string, action: () => Promise<void>) => {
		setRefusal(null);
		setLinking(deskId);
		try {
			await action();
			setConfirming(null);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setLinking(null);
		}
	};
	const linkOf = (desk: Desk) => links.find((link) => link.deskId === desk.id);
	// A link to a desk this window no longer shows is still a link, and still ends here.
	const elsewhere = links.filter((link) => !desks.some((desk) => desk.id === link.deskId));

	return (
		<>
			<section>
				<h3 className="group-title">Desks in this window</h3>
				<div className="grouped">
					<button type="button" className="group-row group-row-add" onClick={onAddDesk}>
						<PlusIcon />
						Add a server
					</button>
					{desks.map((desk) => {
						const link = linkOf(desk);
						const asking = confirming?.deskId === desk.id ? confirming.action : null;
						return (
							<div key={desk.id} className="group-row">
								<span className="group-row-text min-w-0">
									<span className="group-row-title">{desk.name}</span>
									<span className="group-row-detail">
										{[detail(desk), link === undefined ? undefined : linkWords(link)].filter(Boolean).join(" · ")}
									</span>
								</span>
								{desk.kind === "remote" &&
									(asking !== null ? (
										<span className="flex shrink-0 items-center gap-2">
											<button type="button" className="control btn-quiet btn-sm" onClick={() => setConfirming(null)}>
												Keep
											</button>
											<button
												type="button"
												className="control btn btn-sm text-danger"
												disabled={linking === desk.id}
												onClick={() => void act(desk.id, asking === "forget" ? () => forgetDesk(desk.id) : () => unlinkDesk(desk.id))}
											>
												{asking === "forget" ? "Forget" : "Unlink"}
											</button>
										</span>
									) : (
										<span className="flex shrink-0 items-center gap-2">
											{local && link === undefined && desk.state === "open" && (
												<button
													type="button"
													className="control btn-quiet btn-sm"
													disabled={linking === desk.id}
													title="Link both desks' teams"
													onClick={() => void act(desk.id, () => linkDesk(desk.id))}
												>
													{linking === desk.id ? "Linking…" : "Link teammates"}
												</button>
											)}
											{link !== undefined && (
												<button type="button" className="control btn-quiet btn-sm" onClick={() => setConfirming({ deskId: desk.id, action: "unlink" })}>
													Unlink…
												</button>
											)}
											<button type="button" className="control btn-quiet btn-sm" onClick={() => setConfirming({ deskId: desk.id, action: "forget" })}>
												Forget…
											</button>
										</span>
									))}
							</div>
						);
					})}
					{elsewhere.map((link) => (
						<LinkRow key={link.deskId} link={link} busy={linking === link.deskId} onUnlink={() => void act(link.deskId, () => unlinkDesk(link.deskId))} />
					))}
				</div>
				<p className="group-hint">
					Linked desks' teammates see each other and message each other the way teammates on one desk do, each
					under its own grants: the first message to a teammate still asks the person on that teammate's desk.
					Nothing else crosses a link. To also remove this computer from a server, run <code>hotline revoke</code>{" "}
					there.
				</p>
			</section>
			{refusal !== null && <Refusal message={refusal} />}
		</>
	);
}

/** A desk this computer's desk is linked with that this window does not show. */
function LinkRow({ link, busy, onUnlink }: { link: DeskLink; busy: boolean; onUnlink(): void }) {
	return (
		<div className="group-row">
			<span className="group-row-text min-w-0">
				<span className="group-row-title">{link.name}</span>
				<span className="group-row-detail">{linkWords(link)} · not in this window</span>
			</span>
			<button type="button" className="control btn-quiet btn-sm shrink-0" disabled={busy} onClick={onUnlink}>
				Unlink
			</button>
		</div>
	);
}

function detail(desk: Desk): string {
	if (desk.kind === "local") return "Where this window runs";
	switch (desk.state) {
		case "open":
			return "Connected, as owner";
		case "connecting":
			return "Connecting…";
		case "unreachable":
			return desk.error ? `Can't reach it: ${desk.error}` : "Can't reach it right now";
		case "revoked":
			return "No longer recognises this computer";
		default:
			return "Remote";
	}
}
