import { useEffect, useState } from "react";
import type { CookieOffer, TranscriptEvent } from "../generated/contract";
import { wire } from "../wire";
import { Favicon } from "./Favicon";

/**
 * The person's sign-ins lent to a teammate, a row each with Remove. The
 * tape keeps lends across restart; the pane lists even those offscreen in
 * chat. A lend into the teammate's computer is listed with the computer, one
 * into its desk browser's own storage with its secrets: `deskBrowser` picks.
 */
export function CookieLends({
	personaId,
	deskBrowser,
	disabled,
}: {
	personaId: string;
	deskBrowser: boolean;
	disabled: boolean;
}) {
	const [offers, setOffers] = useState<CookieOffer[]>([]);
	const [error, setError] = useState<string | null>(null);
	useEffect(() => {
		let gone = false;
		let revision = 0;
		const refresh = () => {
			const request = ++revision;
			void wire
				.command("computer.cookies.offers", { personaId })
				.then((list) => {
					if (!gone && request === revision) {
						setOffers(Array.isArray(list) ? list : []);
						setError(null);
					}
				})
				.catch((error: unknown) => {
					if (!gone && request === revision)
						setError(error instanceof Error ? error.message : String(error));
				});
		};
		refresh();
		const unsubscribe = wire.subscribe<TranscriptEvent>(
			{ tape: personaId },
			{
				snapshot: refresh,
				event: (event) => {
					if (event.kind === "cookie_offer") refresh();
				},
			},
		);
		return () => {
			gone = true;
			unsubscribe();
		};
	}, [personaId]);
	return (
		<>
			{offers
				.filter(
					(offer) =>
						(offer.status === "lent" || offer.status === "unconfirmed") &&
						(offer.deskBrowser === true) === deskBrowser,
				)
				.map((offer) => (
					<LentRow key={offer.id} offer={offer} disabled={disabled} />
				))}
			{error !== null && (
				<p role="alert" className="group-row text-sm text-danger">
					{error}
				</p>
			)}
		</>
	);
}

/**
 * One sign-in of the person's lent to the teammate, as a row among its
 * settings: the site's icon and name, whose and from which browser, and
 * Remove. The chat's card is for deciding; this is for keeping track.
 */
function LentRow({ offer, disabled }: { offer: CookieOffer; disabled: boolean }) {
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const remove = async () => {
		if (busy || disabled) return;
		setBusy(true);
		setError(null);
		try {
			await wire.command("computer.cookies.remove", { personaId: offer.teammate, id: offer.id });
		} catch (failed) {
			setError(failed instanceof Error ? failed.message : String(failed));
			setBusy(false);
		}
	};
	return (
		<div className="group-row">
			<Favicon site={offer.site} size={16} />
			<span className="group-row-text">
				<span className="group-row-title">{offer.site}</span>
				<span className={`group-row-detail${error !== null ? " text-danger" : ""}`}>
					{error ?? (offer.status === "lent" ? `Signed in as you, from ${offer.browserName}` : `Not confirmed yet, from ${offer.browserName}`)}
				</span>
			</span>
			<button type="button" className="line-key" disabled={busy || disabled} onClick={() => void remove()}>
				Remove
			</button>
		</div>
	);
}
