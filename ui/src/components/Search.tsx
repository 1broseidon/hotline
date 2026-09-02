import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import type { ChapterClose, ChapterSummary, GlobalSearchHit, ThreadSearchHit } from "../generated/contract";
import { matchChord } from "../chords";
import { ChevronDownIcon, ChevronRightIcon, SearchIcon } from "../icons";
import { onTablistKey } from "../ui/Menu";
import { wire, type RosterEntry } from "../wire";

/** How long to wait after a keystroke before asking the index. */
const DEBOUNCE_MS = 150;

type Hit = ThreadSearchHit | GlobalSearchHit;

/**
 * A search over the conversation that is already on screen, in a panel that
 * hangs under the band.
 *
 * Empty, this is the table of contents: chapters are how the tape is
 * divided, so they are what you scan before you type. A query asks the
 * index; chapter hits still outrank messages because a note is the agent's
 * own summary of the thing, not the wording of it. Everywhere drops the
 * teammate filter and names who said it. Arrows walk the list, Enter lands
 * on a hit, Escape and a click outside both close.
 */
export function Search({
	personaId,
	roster,
	onClose,
	onPick,
}: {
	personaId: string;
	roster: RosterEntry[];
	onClose(): void;
	onPick(personaId: string, eventId: string): void;
}) {
	const [query, setQuery] = useState("");
	const [everywhere, setEverywhere] = useState(false);
	const [chapters, setChapters] = useState<ChapterSummary[] | null>(null);
	const [hits, setHits] = useState<Hit[] | null>(null);
	const [truncated, setTruncated] = useState(false);
	/* -1 until an arrow key moves: a list that opens with a row lit is a list
	 * that looks like it has already chosen for you. Enter takes the first. */
	const [active, setActive] = useState(-1);
	const input = useRef<HTMLInputElement>(null);
	const panel = useRef<HTMLDivElement>(null);

	useEffect(() => {
		input.current?.focus({ preventScroll: true });
	}, []);

	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			if (matchChord(event) === "close") {
				event.preventDefault();
				onClose();
				return;
			}
			// Already open: put the caret back rather than letting the browser
			// find-in-page steal a chord the band advertised as ours.
			if (matchChord(event) === "search") {
				event.preventDefault();
				input.current?.focus({ preventScroll: true });
				input.current?.select();
			}
		};
		const away = (event: MouseEvent) => {
			if (panel.current?.contains(event.target as Node)) return;
			onClose();
		};
		window.addEventListener("keydown", onKey);
		document.addEventListener("mousedown", away);
		return () => {
			window.removeEventListener("keydown", onKey);
			document.removeEventListener("mousedown", away);
		};
	}, [onClose]);

	useEffect(() => {
		let cancelled = false;
		setChapters(null);
		void wire.command("chapter.list", { personaId }).then(
			(list) => {
				if (!cancelled) setChapters(list);
			},
			() => {
				if (!cancelled) setChapters([]);
			},
		);
		return () => {
			cancelled = true;
		};
	}, [personaId]);

	const needle = query.trim();

	useEffect(() => {
		setHits(null);
		setTruncated(false);
		setActive(-1);
		if (needle === "") return;
		let cancelled = false;
		const timer = window.setTimeout(() => {
			const ask = everywhere
				? wire.command("search.all", { query: needle })
				: wire.command("search.thread", { personaId, query: needle });
			void ask.then(
				(result) => {
					if (cancelled) return;
					setHits(result.hits);
					setTruncated(result.truncated);
				},
				() => {
					if (cancelled) return;
					setHits([]);
					setTruncated(false);
				},
			);
		}, DEBOUNCE_MS);
		return () => {
			cancelled = true;
			window.clearTimeout(timer);
		};
	}, [needle, everywhere, personaId]);

	const names = new Map(roster.map((entry) => [entry.persona.id, entry.persona.name]));
	const here = names.get(personaId) ?? "Them";
	const searching = needle !== "";
	/* What the arrows walk: the hits, or the chapters when nothing is typed. */
	const rows: { key: string; personaId: string; eventId: string }[] = searching
		? (hits ?? []).map((hit) => ({ key: hitKey(hit, personaId), personaId: personaOf(hit, personaId), eventId: eventOf(hit) }))
		: (chapters ?? []).map((chapter) => ({ key: chapter.id, personaId, eventId: chapter.id }));

	useEffect(() => {
		if (active < 0) return;
		panel.current?.querySelector<HTMLElement>(`[data-row="${active}"]`)?.scrollIntoView({ block: "nearest" });
	}, [active]);

	/* Changing where to look is not leaving the field: the caret goes back. */
	const scope = (wide: boolean) => {
		setEverywhere(wide);
		input.current?.focus({ preventScroll: true });
	};

	const onFieldKey = (event: ReactKeyboardEvent) => {
		if (event.key === "ArrowDown") {
			event.preventDefault();
			setActive((at) => Math.min(rows.length - 1, at + 1));
		} else if (event.key === "ArrowUp") {
			event.preventDefault();
			setActive((at) => Math.max(-1, at - 1));
		} else if (event.key === "Enter") {
			const row = rows[Math.max(0, active)];
			if (!row) return;
			event.preventDefault();
			onPick(row.personaId, row.eventId);
		}
	};

	return (
		<div ref={panel} className="search-panel" role="dialog" aria-label="Search">
			<div className="flex flex-col gap-2 p-2">
				<label className="search-field">
					<SearchIcon className="shrink-0" />
					<input
						ref={input}
						type="search"
						placeholder={everywhere ? "Search every conversation" : "Search this conversation"}
						aria-label={everywhere ? "Search every conversation" : "Search this conversation"}
						value={query}
						spellCheck={false}
						onChange={(event) => setQuery(event.target.value)}
						onKeyDown={onFieldKey}
					/>
				</label>
				<div className="flex items-center justify-between px-1">
					<div className="segmented" role="tablist" aria-label="Where to search" onKeyDown={onTablistKey}>
						<button type="button" role="tab" className="segment" aria-selected={!everywhere} onClick={() => scope(false)}>
							Here
						</button>
						<button type="button" role="tab" className="segment" aria-selected={everywhere} onClick={() => scope(true)}>
							Everywhere
						</button>
					</div>
					{!searching && chapters !== null && chapters.length > 0 && (
						<span className="text-xs text-ink-3">
							{chapters.length} {chapters.length === 1 ? "chapter" : "chapters"}
						</span>
					)}
					{searching && hits !== null && (
						<span className="text-xs text-ink-3">
							{hits.length === 0 ? "No matches" : `${hits.length}${truncated ? "+" : ""} ${hits.length === 1 && !truncated ? "match" : "matches"}`}
						</span>
					)}
				</div>
			</div>

			<div className="min-h-0 flex-1 overflow-y-auto border-t border-line p-1">
				{!searching && (
					<Contents
						chapters={chapters}
						active={active}
						onHover={setActive}
						onPick={(eventId) => onPick(personaId, eventId)}
					/>
				)}
				{searching && hits === null && <p className="px-3 py-2 text-sm text-ink-3">Searching…</p>}
				{searching && hits !== null && hits.length === 0 && (
					<p className="px-3 py-2 text-sm text-ink-3">Nothing matches that yet.</p>
				)}
				{searching &&
					hits?.map((hit, index) => {
						const who = personaOf(hit, personaId);
						return (
							<HitRow
								key={hitKey(hit, personaId)}
								index={index}
								active={index === active}
								hit={hit}
								name={everywhere ? (names.get(who) ?? "?") : undefined}
								here={here}
								onHover={() => setActive(index)}
								onPick={() => onPick(who, eventOf(hit))}
							/>
						);
					})}
				{truncated && (
					<p className="px-3 py-2 text-xs text-ink-3">More matched than are shown. Narrow the words.</p>
				)}
			</div>
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

function HitRow({
	index,
	active,
	hit,
	name,
	here,
	onHover,
	onPick,
}: {
	index: number;
	active: boolean;
	hit: Hit;
	/** Whose tape the hit is from, when the search was everywhere. */
	name: string | undefined;
	/** The teammate on screen, for a hit from their own tape. */
	here: string;
	onHover(): void;
	onPick(): void;
}) {
	const heading =
		hit.kind === "chapter"
			? name !== undefined
				? `${name} · ${hit.title}`
				: hit.title
			: (name ?? (hit.from === "me" ? "You" : here));
	const detail =
		hit.kind === "chapter"
			? hit.status !== undefined
				? statusOf(hit.status)
				: hit.excerpt
			: `${name !== undefined && hit.from === "me" ? "You: " : ""}${hit.excerpt}`;

	return (
		<button
			type="button"
			className="hit"
			data-row={index}
			data-active={active ? "true" : undefined}
			onMouseEnter={onHover}
			onClick={onPick}
		>
			<span className="flex items-baseline gap-2">
				<span className="min-w-0 flex-1 truncate font-medium text-ink">{heading}</span>
				<span className="shrink-0 text-xs text-ink-3">{when(hit.ts)}</span>
			</span>
			{detail !== "" && <span className="block truncate text-xs text-ink-3">{detail}</span>}
		</button>
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
