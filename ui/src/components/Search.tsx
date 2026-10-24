import { useCallback, useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import type { ChapterClose, ChapterSummary, GlobalSearchHit, ThreadSearchHit } from "../generated/contract";
import { chordKeys } from "../chords";
import { ChevronDownIcon, ChevronRightIcon, SearchIcon } from "../icons";
import { Avatar } from "../ui/Avatar";
import { wire, type RosterEntry } from "../wire";

/** How long to wait after a keystroke before asking the index. */
const DEBOUNCE_MS = 150;

type Hit = ThreadSearchHit | GlobalSearchHit;

/** One thing the arrows can land on: a teammate, a message or chapter of one. */
type Row = { key: string; personaId: string; eventId?: string };

/**
 * The team's search: a key that opens a panel holding the field and what it
 * finds. In the rail's dock the panel rises above the dock (`above`), in the
 * faces-only rail it opens beside the rail (`beside`), and with the rail
 * closed the key is in the title bar and it opens under the bar (`under`).
 * What it finds fills the panel as you type:
 * teammates whose name matches, to jump to, then matches from every
 * conversation, grouped by teammate, each opening its conversation at the
 * line. With nothing typed it lists the open conversation's chapters, which
 * is how a tape is divided. Arrows walk the list, Enter lands, Escape clears
 * and lets go, a click elsewhere closes. `ask` changing is the Search chord:
 * the field takes the caret, or the panel opens.
 */
export function TeamSearch({
	roster,
	selectedId,
	placement,
	ask,
	onPick,
}: {
	roster: RosterEntry[];
	/** The conversation on screen, whose chapters an empty query lists. */
	selectedId: string | null;
	placement: "above" | "beside" | "under";
	ask: number;
	onPick(personaId: string, eventId?: string): void;
}) {
	const [query, setQuery] = useState("");
	const [open, setOpen] = useState(false);
	const [chapters, setChapters] = useState<ChapterSummary[] | null>(null);
	const [hits, setHits] = useState<Hit[] | null>(null);
	const [truncated, setTruncated] = useState(false);
	const [searchError, setSearchError] = useState<string | null>(null);
	/* -1 until an arrow key moves: a list that opens with a row lit is a list
	 * that looks like it has already chosen for you. Enter takes the first. */
	const [active, setActive] = useState(-1);
	const input = useRef<HTMLInputElement>(null);
	const box = useRef<HTMLDivElement>(null);
	const needle = query.trim();

	const close = useCallback(() => {
		setOpen(false);
		setQuery("");
		input.current?.blur();
	}, []);

	// The Search chord: take the caret, or open the panel.
	useEffect(() => {
		if (ask === 0) return;
		setOpen(true);
		requestAnimationFrame(() => {
			input.current?.focus({ preventScroll: true });
			input.current?.select();
		});
	}, [ask]);

	useEffect(() => {
		if (!open) return;
		const away = (event: MouseEvent) => {
			if (box.current?.contains(event.target as Node)) return;
			close();
		};
		document.addEventListener("mousedown", away);
		return () => document.removeEventListener("mousedown", away);
	}, [open, close]);

	useEffect(() => {
		if (!open || selectedId === null) {
			setChapters(null);
			return;
		}
		let cancelled = false;
		void wire.command("chapter.list", { personaId: selectedId }).then(
			(result) => {
				if (!cancelled) setChapters(result);
			},
			() => {
				if (!cancelled) setChapters([]);
			},
		);
		return () => {
			cancelled = true;
		};
	}, [open, selectedId]);

	useEffect(() => {
		setHits(null);
		setSearchError(null);
		setTruncated(false);
		setActive(-1);
		if (needle === "") return;
		let cancelled = false;
		const timer = window.setTimeout(() => {
			void wire.command("search.all", { query: needle }).then(
				(result) => {
					if (cancelled) return;
					setHits(result.hits);
					setTruncated(result.truncated);
				},
				(error: unknown) => {
					if (cancelled) return;
					setSearchError(error instanceof Error ? error.message : "Search is unavailable right now. Please try again.");
				},
			);
		}, DEBOUNCE_MS);
		return () => {
			cancelled = true;
			window.clearTimeout(timer);
		};
	}, [needle]);

	const names = new Map(roster.map((entry) => [entry.persona.id, entry.persona.name]));
	const lower = needle.toLowerCase();
	const people = needle === "" ? [] : roster.filter((entry) => entry.persona.name.toLowerCase().includes(lower));
	/* Matches, grouped by whose conversation they are in, in the order the index ranked them. */
	const groups: { personaId: string; hits: Hit[] }[] = [];
	for (const hit of hits ?? []) {
		const who = personaOf(hit, selectedId ?? "");
		const group = groups.find((one) => one.personaId === who);
		if (group) group.hits.push(hit);
		else groups.push({ personaId: who, hits: [hit] });
	}
	const rows: Row[] =
		needle === ""
			? (chapters ?? []).map((chapter) => ({ key: chapter.id, personaId: selectedId ?? "", eventId: chapter.id }))
			: [
					...people.map((entry) => ({ key: `person:${entry.persona.id}`, personaId: entry.persona.id })),
					...groups.flatMap((group) => group.hits.map((hit) => ({ key: hitKey(hit, group.personaId), personaId: group.personaId, eventId: eventOf(hit) }))),
				];

	useEffect(() => {
		if (active < 0) return;
		box.current?.querySelector<HTMLElement>(`[data-row="${active}"]`)?.scrollIntoView({ block: "nearest" });
	}, [active]);

	const pick = (row: Row) => {
		close();
		onPick(row.personaId, row.eventId);
	};

	const onFieldKey = (event: ReactKeyboardEvent) => {
		if (event.key === "Escape") {
			// Ours, not the window's: it would close a pane behind us too.
			event.preventDefault();
			event.stopPropagation();
			close();
		} else if (event.key === "ArrowDown") {
			event.preventDefault();
			setActive((at) => Math.min(rows.length - 1, at + 1));
		} else if (event.key === "ArrowUp") {
			event.preventDefault();
			setActive((at) => Math.max(-1, at - 1));
		} else if (event.key === "Enter") {
			const row = rows[Math.max(0, active)];
			if (!row) return;
			event.preventDefault();
			pick(row);
		}
	};

	const keys = chordKeys("search");
	const field = (
		<label className="search-field">
			<SearchIcon className="shrink-0" />
			<input
				ref={input}
				type="search"
				placeholder="Search"
				aria-label="Search teammates and conversations"
				value={query}
				spellCheck={false}
				onFocus={() => setOpen(true)}
				onChange={(event) => {
					setQuery(event.target.value);
					setOpen(true);
				}}
				onKeyDown={onFieldKey}
			/>
		</label>
	);
	let index = 0;
	const results = (
		<div className="team-results-list">
			{needle === "" && selectedId !== null && (
				<>
					<p className="team-results-head">Chapters · {names.get(selectedId) ?? "this conversation"}</p>
					<Contents chapters={chapters} active={active} onHover={setActive} onPick={(eventId) => pick({ key: eventId, personaId: selectedId, eventId })} />
				</>
			)}
			{needle === "" && selectedId === null && <p className="px-3 py-2 text-sm text-ink-3">Search your teammates and every conversation.</p>}
			{needle !== "" && people.length > 0 && (
				<>
					<p className="team-results-head">Teammates</p>
					{people.map((entry) => {
						const at = index++;
						return (
							<button
								key={entry.persona.id}
								type="button"
								className="hit hit-person"
								data-row={at}
								data-active={at === active ? "true" : undefined}
								onMouseEnter={() => setActive(at)}
								onClick={() => pick({ key: entry.persona.id, personaId: entry.persona.id })}
							>
								<Avatar id={entry.persona.id} name={entry.persona.name} size={20} hash={entry.persona.avatar?.hash} />
								<span className="min-w-0 flex-1 truncate font-medium text-ink">{entry.persona.name}</span>
							</button>
						);
					})}
				</>
			)}
			{needle !== "" && searchError && <p role="alert" className="px-3 py-2 text-sm text-ink-3">{searchError}</p>}
			{needle !== "" && !searchError && hits === null && <p className="px-3 py-2 text-sm text-ink-3">Searching…</p>}
			{needle !== "" && hits !== null && hits.length === 0 && people.length === 0 && <p className="px-3 py-2 text-sm text-ink-3">Nothing matches that yet.</p>}
			{groups.map((group) => (
				<div key={group.personaId}>
					<p className="team-results-head">{names.get(group.personaId) ?? "A teammate"}</p>
					{group.hits.map((hit) => {
						const at = index++;
						return (
							<button
								key={hitKey(hit, group.personaId)}
								type="button"
								className="hit hit-line"
								data-row={at}
								data-active={at === active ? "true" : undefined}
								onMouseEnter={() => setActive(at)}
								onClick={() => pick({ key: hitKey(hit, group.personaId), personaId: group.personaId, eventId: eventOf(hit) })}
							>
								{/* The group says whose; the row is the line itself. */}
								<span className="min-w-0 flex-1 truncate text-ink-2">
									{hit.kind === "chapter" ? hit.title : `${hit.from === "me" ? "You: " : ""}${hit.excerpt}`}
								</span>
								<span className="shrink-0 text-xs text-ink-3">{when(hit.ts)}</span>
							</button>
						);
					})}
				</div>
			))}
			{truncated && <p className="px-3 py-2 text-xs text-ink-3">More matched than are shown. Narrow the words.</p>}
		</div>
	);

	return (
		<div ref={box} className={placement === "under" ? "team-search team-search-under" : "team-search"}>
			<button
				type="button"
				className={placement === "under" ? "control btn-icon" : "rail-search"}
				title={`Search${keys === "" ? "" : ` (${keys})`}`}
				aria-label="Search"
				aria-expanded={open}
				onClick={() => {
					if (open) close();
					else {
						setOpen(true);
						requestAnimationFrame(() => input.current?.focus({ preventScroll: true }));
					}
				}}
			>
				<SearchIcon />
			</button>
			{open && (
				<div className={`team-float team-float-${placement}`} role="dialog" aria-label="Search">
					<div className="p-2">{field}</div>
					<div className="team-float-results">{results}</div>
				</div>
			)}
		</div>
	);
}

/** The empty-query list: every chapter, newest first, note behind a press. */
function Contents({
	chapters,
	active,
	onHover,
	onPick,
}: {
	chapters: ChapterSummary[] | null;
	active: number;
	onHover(index: number): void;
	onPick(eventId: string): void;
}) {
	if (chapters === null) return <p className="px-3 py-2 text-sm text-ink-3">Reading chapters…</p>;
	if (chapters.length === 0) {
		return (
			<p className="px-3 py-2 text-sm text-ink-3">
				No chapters yet. One appears when the agent closes a working context.
			</p>
		);
	}
	return (
		<>
			{chapters.map((chapter, index) => (
				<ChapterRow
					key={chapter.id}
					index={index}
					active={index === active}
					chapter={chapter}
					onHover={() => onHover(index)}
					onPick={() => onPick(chapter.id)}
				/>
			))}
		</>
	);
}

function ChapterRow({
	index,
	active,
	chapter,
	onHover,
	onPick,
}: {
	index: number;
	active: boolean;
	chapter: ChapterSummary;
	onHover(): void;
	onPick(): void;
}) {
	const [noteOpen, setNoteOpen] = useState(false);
	const hasNote = chapter.note !== undefined && chapter.note !== "";
	return (
		<div>
			<button
				type="button"
				className="hit"
				data-row={index}
				data-active={active ? "true" : undefined}
				onMouseEnter={onHover}
				onClick={onPick}
			>
				<span className="flex items-baseline gap-2">
					<span className="min-w-0 flex-1 truncate font-medium text-ink">
						{chapter.title ?? (chapter.endedAt === undefined ? "Current chapter" : "Untitled chapter")}
					</span>
					<span className="shrink-0 text-xs text-ink-3">{when(chapter.startedAt)}</span>
				</span>
				<span className="block truncate text-xs text-ink-3">
					{chapter.messages} {chapter.messages === 1 ? "message" : "messages"}
					{chapter.status !== undefined && ` · ${statusOf(chapter.status)}`}
					{chapter.closedBy !== undefined && ` · closed ${closedByOf(chapter.closedBy)}`}
				</span>
			</button>
			{hasNote && (
				<div className="px-2 pb-1">
					<button
						type="button"
						className="step h-6 min-h-0 py-0 text-xs"
						aria-expanded={noteOpen}
						onClick={() => setNoteOpen((was) => !was)}
					>
						{noteOpen ? <ChevronDownIcon className="text-ink-3" /> : <ChevronRightIcon className="text-ink-3" />}
						Handoff note
					</button>
					{noteOpen && (
						<p className="selectable whitespace-pre-wrap px-2 pb-1 pt-0.5 text-sm text-ink-2">{chapter.note}</p>
					)}
				</div>
			)}
		</div>
	);
}

function personaOf(hit: Hit, fallback: string): string {
	return "personaId" in hit ? hit.personaId : fallback;
}

function eventOf(hit: Hit): string {
	return hit.kind === "chapter" ? hit.chapterId : hit.eventId;
}

function hitKey(hit: Hit, fallback: string): string {
	return `${hit.kind}:${personaOf(hit, fallback)}:${eventOf(hit)}`;
}

function statusOf(status: string): string {
	return status.replace(/-/g, " ");
}

/** How the chapter closed, in the word the person would use. The wire says
 * `user` for a press of New chapter. */
function closedByOf(by: ChapterClose): string {
	if (by === "user") return "by you";
	if (by === "idle") return "on idle";
	if (by === "agent") return "by the agent";
	return "on resume";
}

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const day = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });
const dayYear = new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric" });

/** The clock while it is still today, the date once it is not. */
function when(at: number): string {
	const then = new Date(at);
	const today = new Date();
	const sameDay =
		then.getFullYear() === today.getFullYear() &&
		then.getMonth() === today.getMonth() &&
		then.getDate() === today.getDate();
	if (sameDay) return clock.format(then);
	if (then.getFullYear() === today.getFullYear()) return day.format(then);
	return dayYear.format(then);
}
