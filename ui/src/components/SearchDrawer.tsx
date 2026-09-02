import { useEffect, useRef, useState } from "react";
import type {
	ChapterClose,
	ChapterSummary,
	GlobalSearchHit,
	ThreadSearchHit,
} from "../generated/contract";
import { wire, type RosterEntry } from "../wire";

/** How long to wait after a keystroke before asking the index. */
const DEBOUNCE_MS = 150;

/**
 * A search over the conversation that is already on screen.
 *
 * Empty, this is the table of contents: chapters are how the tape is divided,
 * so they are what you scan before you type. A query asks the index; chapter
 * hits still outrank messages because a note is the agent's own summary of
 * the thing, not the wording of it. Everywhere drops the teammate filter —
 * the index already holds the whole team — and names who said it.
 *
 * The right-hand drawer, the scrim, and the missing close glyph are the
 * reference ThreadsDrawer: Escape and the conversation behind it are already
 * two ways out. The debounced query is GlobalSearch's.
 */
export function SearchDrawer({
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
	const [hits, setHits] = useState<Array<ThreadSearchHit | GlobalSearchHit> | null>(null);
	const [truncated, setTruncated] = useState(false);
	const input = useRef<HTMLInputElement>(null);

	useEffect(() => {
		input.current?.focus({ preventScroll: true });
	}, []);

	useEffect(() => {
		const close = (event: KeyboardEvent) => {
			if (event.key === "Escape") {
				event.preventDefault();
				onClose();
				return;
			}
			// Already open: put the caret back rather than letting the browser
			// find-in-page steal a chord the header advertised as ours.
			if (event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey) {
				if (event.key === "f" || event.code === "KeyF") {
					event.preventDefault();
					input.current?.focus({ preventScroll: true });
				}
			}
		};
		window.addEventListener("keydown", close);
		return () => window.removeEventListener("keydown", close);
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
		if (needle === "") {
			setHits(null);
			setTruncated(false);
			return;
		}
		setHits(null);
		setTruncated(false);
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
	const searching = needle !== "";

	return (
		<div className="absolute inset-0 z-10 flex justify-end" role="dialog" aria-modal="true" aria-label="Search">
			<button
				type="button"
				className="search-scrim"
				aria-label="Close search"
				onClick={onClose}
			/>
			<section className="search-drawer">
				<header className="flex items-center gap-2 border-b border-rule px-3 py-2">
					<input
						ref={input}
						className="field min-w-0 flex-1"
						type="search"
						placeholder={everywhere ? "Search every conversation" : "Search this conversation"}
						aria-label={everywhere ? "Search every conversation" : "Search this conversation"}
						value={query}
						onChange={(event) => setQuery(event.target.value)}
					/>
					<button
						type="button"
						className={`btn-quiet ${everywhere ? "bg-paper-3 text-ink" : ""}`}
						aria-pressed={everywhere}
						onClick={() => setEverywhere((was) => !was)}
					>
						Everywhere
					</button>
				</header>

				<div className="min-h-0 flex-1 overflow-y-auto py-1">
					{!searching && <Contents chapters={chapters} onPick={(eventId) => onPick(personaId, eventId)} />}
					{searching && hits !== null && hits.length === 0 && (
						<p className="px-3 py-2 text-xs text-ink-3">Nothing matches that yet.</p>
					)}
					{searching &&
						hits?.map((hit) => {
							const who = personaOf(hit, personaId);
							return (
								<Hit
									key={hitKey(hit, personaId)}
									hit={hit}
									name={everywhere ? (names.get(who) ?? "?") : undefined}
									onPick={() => onPick(who, eventOf(hit))}
								/>
							);
						})}
					{truncated && (
						<p className="px-3 py-2 text-xs text-ink-3">More matches exist — narrow the words.</p>
					)}
				</div>
			</section>
		</div>
	);
}

/** The empty-query list: every chapter, newest first, note behind a press. */
function Contents({
	chapters,
	onPick,
}: {
	chapters: ChapterSummary[] | null;
	onPick(eventId: string): void;
}) {
	if (chapters === null) {
		return <p className="px-3 py-2 text-xs text-ink-3">Reading chapters…</p>;
	}
	if (chapters.length === 0) {
		return (
			<p className="px-3 py-2 text-xs leading-relaxed text-ink-3">
				No chapters yet. A chapter appears when the agent closes one working context.
			</p>
		);
	}
	return (
		<>
			{chapters.map((chapter) => (
				<div key={chapter.id} className="search-row">
					<button type="button" className="search-hit" onClick={() => onPick(chapter.id)}>
						<span className="flex items-baseline gap-2">
							<span className="min-w-0 flex-1 truncate text-sm font-medium text-ink-2">
								{chapter.title ?? "Untitled chapter"}
							</span>
							<span className="shrink-0 text-xs text-ink-3">{when(chapter.startedAt)}</span>
						</span>
						<span className="block text-left text-xs text-ink-3">
							{chapter.messages} message{chapter.messages === 1 ? "" : "s"}
							{chapter.status !== undefined && ` · ${statusOf(chapter.status)}`}
							{chapter.closedBy !== undefined && ` · ${closedByOf(chapter.closedBy)}`}
						</span>
					</button>
					{chapter.note !== undefined && chapter.note !== "" && (
						<details className="px-3 pb-2">
							<summary className="search-note">Note</summary>
							<p className="mt-1 whitespace-pre-wrap text-xs text-ink-2">{chapter.note}</p>
						</details>
					)}
				</div>
			))}
		</>
	);
}

function Hit({
	hit,
	name,
	onPick,
}: {
	hit: ThreadSearchHit | GlobalSearchHit;
	name: string | undefined;
	onPick(): void;
}) {
	const heading =
		hit.kind === "chapter"
			? name !== undefined
				? `${name} · ${hit.title}`
				: hit.title
			: (name ?? (hit.from === "me" ? "You" : "Them"));
	const detail =
		hit.kind === "chapter"
			? hit.status !== undefined
				? statusOf(hit.status)
				: hit.excerpt
			: `${name !== undefined && hit.from === "me" ? "You: " : ""}${hit.excerpt}`;

	return (
		<button type="button" className="search-hit" onClick={onPick}>
			<span className="flex items-baseline gap-2">
				<span className="min-w-0 flex-1 truncate text-sm font-medium text-ink-2">{heading}</span>
				<span className="shrink-0 text-xs text-ink-3">{when(hit.ts)}</span>
			</span>
			{detail !== "" && <span className="block truncate text-left text-xs text-ink-3">{detail}</span>}
		</button>
	);
}

function personaOf(hit: ThreadSearchHit | GlobalSearchHit, fallback: string): string {
	return "personaId" in hit ? hit.personaId : fallback;
}

function eventOf(hit: ThreadSearchHit | GlobalSearchHit): string {
	return hit.kind === "chapter" ? hit.chapterId : hit.eventId;
}

function hitKey(hit: ThreadSearchHit | GlobalSearchHit, fallback: string): string {
	return `${hit.kind}:${personaOf(hit, fallback)}:${eventOf(hit)}`;
}

function statusOf(status: string): string {
	return status.replace(/-/g, " ");
}

/** How the chapter closed, in the word the person would use. The wire says
 * `user` for a press of New chapter; the spec calls that asked. */
function closedByOf(by: ChapterClose): string {
	if (by === "user") return "asked";
	return by;
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
