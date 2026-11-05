import { useEffect, useState } from "react";
import { wire } from "../../wire";
import { FoldRow } from "./rows";

/**
 * The local addresses this teammate's browser may open without asking: the
 * person's "Always allow" answers, a row each with Remove. Taken back, the
 * teammate's next visit there asks again. What it may reach is a
 * permission, so it sits under Permissions, folded until asked; a teammate
 * with none shows nothing.
 */
export function LocalAddresses({ personaId, disabled }: { personaId: string; disabled: boolean }) {
	const [origins, setOrigins] = useState<string[]>([]);
	const [open, setOpen] = useState(false);
	const [error, setError] = useState<string | null>(null);
	useEffect(() => {
		let gone = false;
		wire
			.command("browser.local.list", { personaId })
			.then((list) => !gone && setOrigins(Array.isArray(list) ? list : []))
			.catch(() => !gone && setOrigins([]));
		return () => {
			gone = true;
		};
	}, [personaId]);
	if (origins.length === 0) return null;
	const remove = (origin: string) => {
		setError(null);
		wire
			.command("browser.local.remove", { personaId, origin })
			.then((list) => setOrigins(Array.isArray(list) ? list : []))
			.catch((failed: unknown) => setError(failed instanceof Error ? failed.message : String(failed)));
	};
	return (
		<>
			<FoldRow title="Local addresses" value={String(origins.length)} open={open} onToggle={() => setOpen((was) => !was)} />
			{open &&
				origins.map((origin) => (
					<div key={origin} className="group-row pl-7">
						<span className="group-row-text">
							<span className="group-row-title font-mono">{origin}</span>
						</span>
						<button type="button" className="line-key" disabled={disabled} title="Ask next time" onClick={() => remove(origin)}>
							Remove
						</button>
					</div>
				))}
			{error !== null && (
				<p role="alert" className="group-row text-sm text-danger">
					{error}
				</p>
			)}
		</>
	);
}
