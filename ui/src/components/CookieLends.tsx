import { useEffect, useState } from "react";
import type { CookieOffer, TranscriptEvent } from "../generated/contract";
import { wire } from "../wire";
import { CookieOfferCard } from "./transcript/CookieOfferCard";

/** The tape keeps lends across restart; the pane lists even those offscreen in chat. */
export function CookieLends({
	personaId,
	disabled,
}: {
	personaId: string;
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
						setOffers(list);
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
					(offer) => offer.status === "lent" || offer.status === "unconfirmed",
				)
				.map((offer) => (
					<CookieOfferCard
						key={offer.id}
						offer={offer}
						teammate="This teammate"
						disabled={disabled}
					/>
				))}
			{error !== null && (
				<p role="alert" className="text-sm text-danger">
					{error}
				</p>
			)}
		</>
	);
}
