import { type RefObject, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { RunningSubagent, ScheduleEntry, ThreadSummary } from "../generated/contract";
import { asideTime, useAsides } from "../asides";
import { usePersonName } from "../room";
import { type ListFilter, type ListItem, filterOf, filtered, grouped, listItems, markOf, rememberFilter, watchFilters } from "../threadList";
import { Scroll } from "../ui/Scroll";
import { wire, type RosterEntry } from "../wire";
import type { ThreadRef } from "./Transcript";

/** The chips over the list, in their order. */
const CHIPS: { filter: ListFilter; word: string }[] = [
	{ filter: "active", word: "Active" },
	{ filter: "all", word: "All" },
	{ filter: "notes", word: "Notes" },
];

/**
 * The teammate's Threads list (docs/design/threads-list.html, idea A): its
 * threads, jobs and notes as rows of one shape, narrowed by the chips over
 * it, in the groups Needs you, Live, Today and Earlier. It opens on Active,
 * where what is earlier than today, routine, or a note already read folds
 * into one row that opens in place. A row opens its thread in the pane; a
 * note opens the thread it came from. Shown with notes in it, the list marks the teammate's notes read for
 * the person (`aside.seen`), which is not the teammate hearing them; what was
 * unread when it opened reads as unread until it closes.
 */
export function ThreadList({
	entry,
	rows,
	jobs,
	runs,
	loaded,
	nameOf,
	opened,
	onOpen,
}: {
	entry: RosterEntry | null;
	/** The teammate's threads as listed, each running or not as the roster says. */
	rows: ThreadSummary[];
	jobs: ScheduleEntry[];
	/** The teammate's runs going now, as the roster says: what each did last. */
	runs: readonly RunningSubagent[];
	loaded: boolean;
	nameOf(personaId: string): string | undefined;
	/** The row a thread was last opened from. */
	opened: RefObject<string | null>;
	onOpen(open: ThreadRef): void;
}) {
	const root = useRef<HTMLDivElement>(null);
	const personaId = entry?.persona.id;
	const asides = useAsides(personaId, entry);
	const personName = usePersonName();
	const filter = useSyncExternalStore(watchFilters, () => (personaId === undefined ? "active" : filterOf(personaId)));
	const now = useNow(jobs.some((job) => job.endedAt === undefined) ? 1_000 : 30_000);

	/* What was unread when the list opened, so marking it read does not move it. */
	const fresh = useRef(new Set<string>());
	for (const aside of asides ?? []) if (aside.seen === undefined) fresh.current.add(aside.id);
	const showsNotes = filter === "all" || filter === "notes";
	const anyUnseen = (asides ?? []).some((aside) => aside.seen === undefined);
	useEffect(() => {
		if (personaId !== undefined && showsNotes && anyUnseen) void wire.command("aside.seen", { personaId }).catch(() => undefined);
	}, [personaId, showsNotes, anyUnseen]);

	const items = useMemo(() => {
		const read = (asides ?? []).map((aside) => {
			if (!fresh.current.has(aside.id)) return aside;
			const { seen: _seen, ...unread } = aside;
			return unread;
		});
		return listItems({ rows, runs, jobs, asides: read, listedFor: personaId ?? "", nameOf, personName, now });
	}, [rows, runs, jobs, asides, personaId, nameOf, personName, now]);
	const shown = filtered(items, filter);
	const active = filter === "active";
	const groups = grouped(shown, now, active);
	/* Earlier and routine, opened in place; a chip pressed folds it again. */
	const [unfolded, setUnfolded] = useState(false);
	const [foldedFor, setFoldedFor] = useState(filter);
	if (foldedFor !== filter) {
		setFoldedFor(filter);
		setUnfolded(false);
	}

	// Opening the list puts focus in it: back from a thread, on the row it was opened from, else on the chip pressed.
	const ready = loaded && asides !== undefined;
	const focused = useRef(false);
	useEffect(() => {
		if (focused.current || !ready) return;
		focused.current = true;
		// Back to a row that was folded away, the fold is open.
		if (groups.earlier.some((item) => item.id === opened.current)) setUnfolded(true);
		const dock = root.current?.closest(".dock");
		(dock?.querySelector<HTMLElement>("[data-restore]") ?? dock?.querySelector<HTMLElement>(".list-chips [aria-pressed=true]"))?.focus();
	}, [ready, groups.earlier, opened]);

	const row = (item: ListItem) => <Row key={item.id} item={item} now={now} opened={opened} onOpen={onOpen} />;

	if (entry === null) return <p className="px-4 py-3 text-sm text-ink-3">Pick a teammate to see their threads.</p>;
	return (
		<>
			<fieldset className="list-chips chips" aria-label="Show">
				{CHIPS.map((chip) => (
					<button
						key={chip.filter}
						type="button"
						className="control btn-quiet btn-sm"
						aria-pressed={filter === chip.filter}
						onClick={() => rememberFilter(entry.persona.id, chip.filter)}
					>
						{chip.word}
					</button>
				))}
			</fieldset>
			<Scroll>
				<div ref={root} className="px-2 pb-3">
					{!ready ? null : items.length === 0 ? (
						<p className="px-2 py-3 text-sm text-ink-3">No threads with {entry.persona.name} yet.</p>
					) : shown.length === 0 ? (
						<p className="px-2 py-3 text-sm text-ink-3">Nothing here.</p>
					) : (
						<>
							<Group title="Needs you" count={groups.needs.length} />
							{groups.needs.map(row)}
							<Group title="Live" count={groups.live.length} />
							{groups.live.map(row)}
							<Group title="Today" items={groups.today} />
							{groups.today.map(row)}
							{active && groups.earlier.length > 0 ? (
								<>
									<button type="button" className="list-fold" aria-expanded={unfolded} onClick={() => setUnfolded((was) => !was)}>
										<span>Earlier and routine · {groups.earlier.length}</span>
										<span>{unfolded ? "Hide" : "Show"}</span>
									</button>
									{unfolded && groups.earlier.map(row)}
								</>
							) : (
								<>
									<Group title="Earlier" items={groups.earlier} />
									{groups.earlier.map(row)}
								</>
							)}
						</>
					)}
				</div>
			</Scroll>
		</>
	);
}

/**
 * A group's heading, when it has rows. Only Needs you and Live are counted:
 * how many wait and how many run are the numbers worth a glance.
 */
function Group({ title, count, items }: { title: string; count?: number; items?: ListItem[] }) {
	if ((count ?? items?.length ?? 0) === 0) return null;
	return (
		<h3 className="list-group">
			<span>{title}</span>
			{count !== undefined && <span>{count}</span>}
		</h3>
	);
}

/** A row: the kind's mark, the title over its one line, and the time, with the beat while it runs. */
function Row({ item, now, opened, onOpen }: { item: ListItem; now: number; opened: RefObject<string | null>; onOpen(open: ThreadRef): void }) {
	const mark = markOf(item.kind);
	const body = (
		<>
			<span className="list-mark" title={mark.word} aria-hidden="true">
				{mark.glyph}
			</span>
			<span className="min-w-0 flex-1">
				<span className="list-title">
					<span className="list-title-text">{item.title}</span>
					{item.notes > 0 && (
						<span className="list-notes" title={`${item.notes} ${item.notes === 1 ? "note" : "notes"}`}>
							✎ {item.notes}
						</span>
					)}
				</span>
				<span className="list-line">
					{item.line}
					{item.kind === "note" && item.open !== undefined && " ›"}
				</span>
			</span>
			<span className="list-when">
				{asideTime(item.at, now)}
				{item.live && <span aria-hidden="true" className="dock-dot beat" data-state="running" />}
			</span>
		</>
	);
	const label = `${mark.word}: ${item.title}, ${item.line}`;
	const state = item.waiting ? "waiting" : item.live ? "live" : item.ended ? "ended" : undefined;
	const open = item.open;
	if (open === undefined) {
		return (
			<div className="list-row" data-thread={item.id} data-kind={item.kind} data-level={item.level}>
				{body}
			</div>
		);
	}
	return (
		<button
			type="button"
			className="list-row"
			data-thread={item.id}
			data-kind={item.kind}
			data-state={state}
			data-level={item.level}
			data-restore={opened.current === item.id || undefined}
			aria-label={label}
			onClick={() => {
				opened.current = item.id;
				onOpen(open);
			}}
		>
			{body}
		</button>
	);
}

/** The clock the list's times and countdowns read, ticking every `every` ms. */
function useNow(every: number): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const tick = window.setInterval(() => setNow(Date.now()), every);
		return () => window.clearInterval(tick);
	}, [every]);
	return now;
}
