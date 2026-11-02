import type { ThreadId } from "../generated/contract";
import { shownState } from "../activity";
import type { Person } from "../avatars";
import { Avatar } from "../ui/Avatar";
import { MenuButton, type MenuEntry } from "../ui/Menu";
import { wire, type RosterEntry } from "../wire";

/**
 * What a colleague does when looped in, by what it is doing now: an idle one
 * is woken by the nudge, and a busy one answers from a work thread of its own
 * at once, as any question to it is answered.
 */
function loopInState(entry: RosterEntry): string {
	return shownState(entry.session) === "thinking" ? "Working · a thread answers" : "Idle · a nudge wakes it";
}

/**
 * Loop in…, in a work thread's header: the colleagues on this desk, each with
 * what it is doing, and choosing one leaves it a nudge from this thread and
 * asks it for one answer here (`thread.loop_in`). The person's act is the
 * grant, so anyone may be looped in; the thread draws who was.
 */
export function LoopIn({
	thread,
	owner,
	roster,
	people,
	onRefused,
}: {
	thread: ThreadId;
	/** The thread's own teammate, who is in it already. */
	owner: string;
	roster: RosterEntry[];
	people: ReadonlyMap<string, Person>;
	onRefused(error: unknown): void;
}) {
	const colleagues = roster.filter((entry) => entry.persona.id !== owner && entry.persona.node === undefined);
	if (colleagues.length === 0) return null;
	const entries: MenuEntry[] = [
		{ kind: "heading", text: "Loop in", detail: "A nudge, then one answer here" },
		...colleagues.map((entry): MenuEntry => ({
			kind: "item",
			id: entry.persona.id,
			text: entry.persona.name,
			detail: loopInState(entry),
			icon: <Avatar id={entry.persona.id} name={entry.persona.name} size={20} hash={people.get(entry.persona.id)?.hash} />,
			onSelect: () => void wire.command("thread.loop_in", { thread, personaId: entry.persona.id }).catch(onRefused),
		})),
	];
	return (
		<MenuButton className="control btn-quiet px-2 text-sm" label="Loop in a colleague" title="Loop someone in" entries={entries}>
			Loop in…
		</MenuButton>
	);
}
