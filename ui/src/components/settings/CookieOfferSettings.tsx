import { useCallback, useEffect, useState } from "react";
import type { CookieOfferBlock } from "../../generated/contract";
import { wire } from "../../wire";

export function CookieOfferSettings() {
	const [blocked, setBlocked] = useState<CookieOfferBlock[]>([]);
	const [error, setError] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const refresh = useCallback(async () => {
		setBlocked(await wire.command("computer.cookies.blocked", {}));
	}, []);
	useEffect(() => {
		void refresh().catch((error: unknown) =>
			setError(error instanceof Error ? error.message : String(error)),
		);
	}, [refresh]);
	const undo = async (block: CookieOfferBlock) => {
		setBusy(true);
		setError(null);
		try {
			await wire.command("computer.cookies.reset", {
				personaId: block.teammate,
				site: block.site,
			});
			await refresh();
		} catch (error) {
			setError(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};
	return (
		<section>
			<h3 className="group-title">Browser sign-in offers</h3>
			<p className="group-hint">
				Every lend asks first. Sites you chose never to offer are listed here
				for each teammate.
			</p>
			{blocked.length === 0 && (
				<p className="text-sm text-ink-3">No sites hidden.</p>
			)}
			<div className="grouped">
				{blocked.map((block) => (
					<div className="group-row" key={`${block.teammate}/${block.site}`}>
						<span className="group-row-text">
							{block.name} · {block.site}
						</span>
						<button
							type="button"
							className="control btn"
							disabled={busy}
							onClick={() => void undo(block)}
						>
							Ask again
						</button>
					</div>
				))}
			</div>
			{error !== null && (
				<p role="alert" className="text-sm text-danger">
					{error}
				</p>
			)}
		</section>
	);
}
