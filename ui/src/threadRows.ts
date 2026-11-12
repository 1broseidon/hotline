import { useCallback, useEffect, useMemo, useState } from "react";
import type { ScheduleEntry, ThreadSummary } from "./generated/contract";
import { type LinkEvent, withLinks } from "./dock";
import { dmOf, useThread } from "./tape";
import { wire, type RosterEntry } from "./wire";

/*
 * What a teammate's Threads list is read from, for the right-hand pane's list
 * and the composer's count of what runs (task-437), so the two read the same
 * rows and can never disagree.
 */

/** Each teammate's last list, shown at once on the way back while it is read again. */
const lastLists = new Map<string, ThreadSummary[]>();

/** A read under way, by teammate and what changed: the pane and the composer asking at once share it. */
const reading = new Map<string, Promise<ThreadSummary[]>>();

function readList(personaId: string, key: string): Promise<ThreadSummary[]> {
	const asked = reading.get(key);
	if (asked !== undefined) return asked;
	const read = wire.command("thread.list", { personaId }).finally(() => reading.delete(key));
	reading.set(key, read);
	return read;
}

/**
 * The teammate's threads of every kind: what `thread.list` says, read again
 * whenever a thread starts, ends, parks or changes hands, a turn in one starts
 * or ends (a pair's included: the roster's `workingPairs`), or a card lands, and brought up to what the links on the
 * conversation say in between.
 */
export function useThreadRows(entry: RosterEntry | null): { rows: ThreadSummary[]; list: ThreadSummary[] | undefined; reload(): void } {
	const personaId = entry?.persona.id;
	const [list, setList] = useState<ThreadSummary[] | undefined>(() => (personaId === undefined ? undefined : lastLists.get(personaId)));
	const [revision, setRevision] = useState(0);
	const { events } = useThread(personaId === undefined ? null : dmOf(personaId));
	const links = useMemo(() => events.filter((event): event is LinkEvent => event.kind === "link"), [events]);
	// A thread that started, parked or ended, or whose handoff ended, is a link rewritten; a turn starting or a card landing is the roster's word.
	const signature =
		links.map((link) => `${link.thread}:${link.state}${link.handoffEnded === true ? ":ended" : ""}`).join(",") +
		"|" +
		(entry?.sides ?? []).map((side) => `${side.sideId}:${side.working ? 1 : 0}`).join(",") +
		"|" +
		(entry?.subagents ?? []).map((run) => run.runId).join(",") +
		"|" +
		(entry?.workingJobs ?? []).join(",") +
		"|" +
		(entry?.workingCalls ?? []).join(",") +
		"|" +
		(entry?.workingPairs ?? []).join(",") +
		(entry?.waiting === true ? "|waiting" : "");
	// Read again when the signature or the revision says the threads changed; the key shares a read already under way.
	useEffect(() => {
		if (personaId === undefined) return;
		let cancelled = false;
		void readList(personaId, `${personaId}\u0000${signature}\u0000${revision}`)
			.then((next) => {
				lastLists.set(personaId, next);
				if (!cancelled) setList(next);
			})
			.catch(() => !cancelled && setList((was) => was ?? []));
		return () => {
			cancelled = true;
		};
	}, [personaId, signature, revision]);
	// A different teammate starts from what was last read for them, if anything; set during render so the old list never flashes.
	const [shownFor, setShownFor] = useState(personaId);
	if (shownFor !== personaId) {
		setShownFor(personaId);
		setList(personaId === undefined ? undefined : lastLists.get(personaId));
	}
	const rows = useMemo(() => withLinks(list ?? [], links, personaId ?? ""), [list, links, personaId]);
	return { rows, list, reload: useCallback(() => setRevision((one) => one + 1), []) };
}

/** This teammate's jobs as the schedules view sends them, ended ones included. */
export function useJobs(personaId: string | undefined): ScheduleEntry[] {
	const [entries, setEntries] = useState<ScheduleEntry[]>([]);
	useEffect(() => {
		setEntries([]);
		if (personaId === undefined) return;
		return wire.subscribe<ScheduleEntry>(
			{ schedules: personaId },
			{ snapshot: (next) => setEntries(next as ScheduleEntry[]), event: () => {}, removed: () => setEntries([]) },
		);
	}, [personaId]);
	return entries;
}

/** A teammate's name; one on a linked desk is in no roster here, and its pair's row names it ("Lou on grizzly"). */
export function nameIn(roster: readonly RosterEntry[], rows: readonly ThreadSummary[], personaId: string): string | undefined {
	return roster.find((one) => one.persona.id === personaId)?.persona.name ?? rows.find((row) => row.withPersonaId === personaId)?.withName;
}
