import { useState } from "react";
import { forgetDesk } from "../native";
import { useDesks, type Desk } from "../desks";
import { Refusal } from "../ui/Refusal";

/**
 * The desks this window reaches (BRO-145): this computer's, and the servers
 * it is paired with as owner. Forgetting a server stops reaching it from
 * here; the server still lists this computer until it is revoked there.
 */
export function DesksSection({ onAddDesk }: { onAddDesk(): void }) {
	const desks = useDesks();
	const [refusal, setRefusal] = useState<string | null>(null);
	const [confirming, setConfirming] = useState<string | null>(null);

	const forget = async (desk: Desk) => {
		setRefusal(null);
		try {
			await forgetDesk(desk.id);
			setConfirming(null);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		}
	};

	return (
		<>
			<section>
				<h3 className="group-title">Desks in this window</h3>
				<div className="grouped">
					{desks.map((desk) => (
						<div key={desk.id} className="group-row">
							<span className="group-row-text min-w-0">
								<span className="group-row-title">{desk.name}</span>
								<span className="group-row-detail">{detail(desk)}</span>
							</span>
							{desk.kind === "remote" &&
								(confirming === desk.id ? (
									<span className="flex shrink-0 items-center gap-2">
										<button type="button" className="control btn btn-sm" onClick={() => setConfirming(null)}>
											Keep
										</button>
										<button type="button" className="control btn btn-sm btn-danger" onClick={() => void forget(desk)}>
											Forget
										</button>
									</span>
								) : (
									<button type="button" className="control btn btn-sm shrink-0" onClick={() => setConfirming(desk.id)}>
										Forget…
									</button>
								))}
						</div>
					))}
				</div>
				<p className="group-hint">
					To also remove this computer from a server, run <code>hotline revoke</code> there.
				</p>
			</section>
			<div className="flex justify-start">
				<button type="button" className="control btn" onClick={onAddDesk}>
					Add a server…
				</button>
			</div>
			{refusal !== null && <Refusal message={refusal} />}
		</>
	);
}

function detail(desk: Desk): string {
	if (desk.kind === "local") return "This computer";
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
