import { isWorking } from "../activity";
import type { RosterEntry } from "../wire";

/**
 * Whether the teammate on the call is working off it: its own conversation's
 * turn, or a work thread of its own with a turn going. A call with the desk
 * counts any teammate's work.
 */
export function callWorking(roster: readonly Pick<RosterEntry, "persona" | "session" | "sides">[], personaId: string | undefined): boolean {
	const rows = personaId === undefined ? roster : roster.filter((row) => row.persona.id === personaId);
	return rows.some((row) => {
		return isWorking(row.session) || (row.sides ?? []).some((side) => side.working);
	});
}
