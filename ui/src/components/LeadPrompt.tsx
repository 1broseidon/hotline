import { useEffect, useState } from "react";
import type { RosterEntry } from "../wire";

/*
 * The desks whose prompt was put off with Not now. Kept outside the window's
 * state so switching desks does not ask again; a launch starts it empty,
 * and so does the next desk call.
 */
const putOff = new Set<string>();

/**
 * A desk of two or more with no lead asks the person to pick one. The core
 * never picks among two or more, so until someone does, this says so and
 * offers the teammate who has been on the team longest; any teammate's
 * menu makes another. Not now puts it off until the next desk call or the
 * next launch. A lead named, here or anywhere, ends that, so losing the
 * lead later asks again.
 */
export function LeadPrompt({
	deskId,
	roster,
	deskCalling,
	onLead,
}: {
	deskId: string;
	roster: RosterEntry[];
	/** A call to the desk is up: it ends a Not now. */
	deskCalling: boolean;
	onLead(personaId: string): void;
}) {
	const [notNow, setNotNow] = useState(() => putOff.has(deskId));
	const led = roster.some((entry) => entry.lead === true);
	useEffect(() => {
		if (!led) return;
		putOff.delete(deskId);
		setNotNow(false);
	}, [led, deskId]);
	useEffect(() => {
		if (!deskCalling) return;
		putOff.clear();
		setNotNow(false);
	}, [deskCalling]);
	if (notNow || led || roster.length < 2) return null;
	const suggested = longestOnTeam(roster);
	return (
		<section aria-label="Pick a team lead" className="flex shrink-0 items-center gap-2 rounded-md bg-raised px-4 py-1.5 text-sm text-ink">
			<span className="min-w-0 flex-1">Pick a team lead. {suggested.persona.name} has been on the team longest.</span>
			<button type="button" className="control btn btn-sm" onClick={() => onLead(suggested.persona.id)}>
				Make {suggested.persona.name} lead
			</button>
			<button
				type="button"
				className="control btn-quiet btn-sm"
				onClick={() => {
					putOff.add(deskId);
					setNotNow(true);
				}}
			>
				Not now
			</button>
		</section>
	);
}

/** The teammate made first; the roster's order breaks a tie. */
function longestOnTeam(roster: RosterEntry[]): RosterEntry {
	return roster.reduce((oldest, entry) => (entry.persona.createdAt < oldest.persona.createdAt ? entry : oldest));
}
