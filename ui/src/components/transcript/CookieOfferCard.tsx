import { useEffect, useState } from "react";
import type { CookieOffer, CookieOfferAnswer } from "../../generated/contract";
import { wire } from "../../wire";

export function CookieOfferCard({
	offer,
	teammate,
	disabled = false,
}: {
	offer: CookieOffer;
	teammate: string;
	disabled?: boolean;
}) {
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	// A settlement from either seat releases the button for the new action.
	// biome-ignore lint/correctness/useExhaustiveDependencies: status deliberately resets local submission state.
	useEffect(() => {
		setBusy(false);
		setError(null);
	}, [offer.status]);
	const act = async (answer?: CookieOfferAnswer) => {
		if (busy || disabled) return;
		setBusy(true);
		setError(null);
		try {
			if (answer === undefined)
				await wire.command("computer.cookies.remove", {
					personaId: offer.teammate,
					id: offer.id,
				});
			else
				await wire.command("computer.cookies.answer", {
					personaId: offer.teammate,
					id: offer.id,
					answer,
				});
		} catch (error) {
			setError(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};
	const pending = offer.status === "pending";
	return (
		<div className={`card mt-3${pending ? " card-live" : ""}`}>
			{offer.status === "pending" ? (
				<>
					<p className="selectable">
						Use your existing cookie from {offer.browserName} for {offer.site}?
					</p>
					<p className="mt-1 text-sm text-ink-2">
						{teammate} will be signed in as you.
					</p>
					<p className="mt-1 text-sm text-ink-3">
						{offer.browserName} · {offer.profileName}
					</p>
					<div className="mt-2.5 flex flex-wrap gap-1.5">
						<button
							type="button"
							className="control btn-primary"
							disabled={busy || disabled}
							onClick={() => void act("use")}
						>
							Use
						</button>
						<button
							type="button"
							className="control btn"
							disabled={busy || disabled}
							onClick={() => void act("not_now")}
						>
							Not now
						</button>
						<button
							type="button"
							className="control btn"
							disabled={busy || disabled}
							onClick={() => void act("never")}
						>
							Never for this site
						</button>
					</div>
				</>
			) : offer.status === "lent" || offer.status === "unconfirmed" ? (
				<div className="flex flex-wrap items-center gap-2">
					<p className="selectable">
						{offer.status === "lent"
							? "Signed in as you"
							: "Sign-in unconfirmed"}{" "}
						on {offer.site}, from {offer.browserName}
					</p>
					<button
						type="button"
						className="control btn"
						disabled={busy || disabled}
						onClick={() => void act()}
					>
						Remove
					</button>
				</div>
			) : (
				<p className="text-sm text-ink-3">
					{offer.site} ·{" "}
					{
						{
							declined: "Not now",
							never: "Never for this site · Undo in Settings → Secrets",
							expired: "Offer expired",
							removed: "Sign-in removed from the computer",
						}[offer.status]
					}
				</p>
			)}
			{error !== null && (
				<p role="alert" className="mt-2 text-sm text-danger">
					{error}
				</p>
			)}
		</div>
	);
}
