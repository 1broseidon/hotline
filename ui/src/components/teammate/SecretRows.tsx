import { useCallback, useState } from "react";
import type { SecretGrant } from "../../generated/contract";
import { isDesktop, platform } from "../../native";
import { PasskeyArm } from "../PasskeyArm";
import { TeammateSecrets } from "../Secrets";
import { Cookies } from "./Cookies";
import { FoldRow } from "./rows";

/**
 * Whether the desk hands a teammate's desk shell its secrets. Not yet: the
 * desk shell's key proxy is W2.3 of the protected-secrets plan, and until
 * it lands nothing would be behind a grant's choice of Shell.
 */
const DESK_SHELL_TAKES_SECRETS: boolean = false;

/**
 * The stored secrets this teammate is granted, as its own section: the
 * grant is the teammate's, and whichever shell and browser it has uses it.
 * One fold, closed until asked, because opening it reads the keychain's
 * list. Without a computer, the desk's own browser (the Mac app's) types
 * its logins and signs in with its passkeys, and a passkey for it is made
 * here; with neither, nothing uses the ticks yet, and the section says so
 * rather than let a tick look like it did something. A passkey for the
 * teammate's browser, its desk browser's or its computer's, is made here.
 * Under it, the person's cookies shared with that browser: one list for
 * either, in sight without opening anything. With a computer, a grant may
 * reach its computer, its desk shell or both.
 */
export function SecretRows({
	personaId,
	teammate,
	grants,
	computer,
	asPerson,
	disabled,
	onChange,
}: {
	personaId: string;
	teammate: string;
	grants: SecretGrant[];
	computer: boolean;
	/** Its programs run as the person: the whole machine, or a harness. */
	asPerson: boolean;
	disabled: boolean;
	onChange(grants: SecretGrant[]): void;
}) {
	const granted = grants.length;
	const [open, setOpen] = useState(false);
	const deskBrowser = !computer && isDesktop() && ["macos", "linux", "windows"].includes(platform());
	// Only the Mac app's desk browser signs with a passkey; elsewhere a page's
	// passkey call is refused, so there is nothing to arm.
	const deskSigner = deskBrowser && platform() === "macos";
	// The room ticks a passkey it stores, and the roster brings the tick.
	const stored = useCallback(() => {}, []);

	return (
		<section>
			<h3 className="label">Secrets</h3>
			<div className="grouped">
				<FoldRow title="Logins and passkeys" value={granted === 0 ? "none given" : `${granted} given`} open={open} onToggle={() => setOpen((was) => !was)} />
				{open && <TeammateSecrets grants={grants} bothPlaces={computer && DESK_SHELL_TAKES_SECRETS} asPerson={asPerson} disabled={disabled} onChange={onChange} />}
				{deskSigner && <PasskeyArm personaId={personaId} teammate={teammate} desk disabled={disabled} onStored={stored} />}
				{computer && <PasskeyArm personaId={personaId} teammate={teammate} disabled={disabled} onStored={stored} />}
			</div>
			{open && <p className="group-hint">Kept in Settings › Secrets. A passkey made for this teammate is given by itself.</p>}
			{deskBrowser && granted > 0 && <p className="group-hint">{deskSigner ? "Its browser types these logins and signs in with these passkeys." : "Its browser types these logins."} A variable needs a computer's shell.</p>}
			{!computer && !deskBrowser && granted > 0 && <p className="group-hint">Its computer's shell and browser use these. It has no computer, so nothing does yet.</p>}
			{(computer || deskBrowser) && <Cookies personaId={personaId} disabled={disabled} />}
		</section>
	);
}
