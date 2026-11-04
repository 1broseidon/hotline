import { useEffect, useState } from "react";
import { TeammateSecrets } from "../Secrets";
import { FoldRow } from "./rows";

/**
 * The stored secrets this teammate is granted, as its own section: the
 * grant is the teammate's, and whichever shell and browser it has uses it.
 * One fold, closed until asked, because opening it reads the keychain's
 * list. Without a computer nothing uses the ticks yet, and the section
 * says so rather than let a tick look like it did something.
 */
export function SecretRows({
	personaId,
	granted,
	computer,
	disabled,
	onChange,
}: {
	personaId: string;
	granted: string[];
	computer: boolean;
	disabled: boolean;
	onChange(granted: string[]): void;
}) {
	const [open, setOpen] = useState(false);

	// Another teammate's pane opens closed.
	useEffect(() => {
		setOpen(false);
	}, [personaId]);

	return (
		<section>
			<h3 className="label">Secrets</h3>
			<div className="grouped">
				<FoldRow title="Granted" value={granted.length === 0 ? "none ticked" : `${granted.length} ticked`} open={open} onToggle={() => setOpen((was) => !was)} />
				{open && <TeammateSecrets granted={granted} disabled={disabled} onChange={onChange} />}
			</div>
			{!computer && granted.length > 0 && <p className="group-hint">Its computer's shell and browser use these. It has no computer, so nothing does yet.</p>}
		</section>
	);
}
