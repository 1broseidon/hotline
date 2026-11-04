import { useCallback, useState } from "react";
import { isDesktop, platform } from "../../native";
import { PasskeyArm } from "../PasskeyArm";
import { TeammateSecrets } from "../Secrets";
import { FoldRow } from "./rows";

/**
 * The stored secrets this teammate is granted, as its own section: the
 * grant is the teammate's, and whichever shell and browser it has uses it.
 * One fold, closed until asked, because opening it reads the keychain's
 * list. Without a computer, the desk's own browser (the Mac app's) types
 * its logins and signs in with its passkeys, and a passkey for it is made
 * here; with neither, nothing uses the ticks yet, and the section says so
 * rather than let a tick look like it did something.
 */
export function SecretRows({
	personaId,
	teammate,
	granted,
	computer,
	disabled,
	onChange,
}: {
	personaId: string;
	teammate: string;
	granted: string[];
	computer: boolean;
	disabled: boolean;
	onChange(granted: string[]): void;
}) {
	const [open, setOpen] = useState(false);
	const deskBrowser = !computer && isDesktop() && platform() === "macos";
	// The room ticks a passkey it stores, and the roster brings the tick.
	const stored = useCallback(() => {}, []);

	return (
		<section>
			<h3 className="label">Secrets</h3>
			<div className="grouped">
				<FoldRow title="Granted" value={granted.length === 0 ? "none ticked" : `${granted.length} ticked`} open={open} onToggle={() => setOpen((was) => !was)} />
				{open && <TeammateSecrets granted={granted} disabled={disabled} onChange={onChange} />}
				{deskBrowser && <PasskeyArm personaId={personaId} teammate={teammate} desk disabled={disabled} onStored={stored} />}
			</div>
			{deskBrowser && granted.length > 0 && <p className="group-hint">Its browser types these logins and signs in with these passkeys. A variable needs a computer's shell.</p>}
			{!computer && !deskBrowser && granted.length > 0 && <p className="group-hint">Its computer's shell and browser use these. It has no computer, so nothing does yet.</p>}
		</section>
	);
}
