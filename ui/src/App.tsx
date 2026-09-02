import { useCallback, useEffect, useState } from "react";
import type { ConfigChoice } from "./generated/contract";
import { noticeRoster, setWindowTitle, watchNotificationClicks } from "./notify";
import { useTape } from "./tape";
import { wire, type Connection, type RosterEntry } from "./wire";
import { ChatHeader } from "./components/ChatHeader";
import { Composer } from "./components/Composer";
import { NewTeammate } from "./components/NewTeammate";
import { Rail } from "./components/Rail";
import { SearchDrawer } from "./components/SearchDrawer";
import { Settings } from "./components/Settings";
import { Teammate } from "./components/Teammate";
import { Transcript, type ReplyTarget } from "./components/Transcript";

type SheetKind = "new-teammate" | "settings" | "teammate" | null;

export function App() {
	const [connection, setConnection] = useState<Connection>("connecting");
	const [roster, setRoster] = useState<RosterEntry[]>([]);
	const [seen, setSeen] = useState<Record<string, number>>(loadSeen);
	const [models, setModels] = useState<ConfigChoice[]>([]);
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const [sheet, setSheet] = useState<SheetKind>(null);
	const [searchOpen, setSearchOpen] = useState(false);
	const [focus, setFocus] = useState<{ eventId: string; at: number } | null>(null);

	useEffect(() => {
		wire.connect();
		return wire.onConnection(setConnection);
	}, []);

	useEffect(() => {
		return wire.subscribe<RosterEntry>(
			{ view: "roster" },
			{
				snapshot: setRoster,
				event: (entry) =>
					setRoster((known) => {
						const at = known.findIndex((one) => one.persona.id === entry.persona.id);
						if (at === -1) return [...known, entry];
						const next = known.slice();
						next[at] = entry;
						return next;
					}),
				removed: (personaId) =>
					setRoster((known) => known.filter((one) => one.persona.id !== personaId)),
			},
		);
	}, []);

	/* The room's models are asked for once a socket is up, and again after a
	 * reconnect: a key added on another seat changes the answer. */
	useEffect(() => {
		if (connection !== "open") return;
		wire
			.command("models.list", {})
			.then(setModels)
			.catch(() => setModels([]));
	}, [connection]);

	const selected = roster.find((one) => one.persona.id === selectedId) ?? null;

	/* Opening a teammate, or sitting on one while a new line lands, is what
	 * "shown" means. The rail then has a ts to compare against, so it does
	 * not have to open every tape. */
	useEffect(() => {
		if (selectedId === null || selected?.latest == null) return;
		const latest = selected.latest;
		setSeen((known) => {
			if (known[selectedId] === latest) return known;
			const next = { ...known, [selectedId]: latest };
			saveSeen(next);
			return next;
		});
	}, [selectedId, selected?.latest]);

	useEffect(() => {
		if (sheet === "teammate" && selected === null) setSheet(null);
	}, [sheet, selected]);

	useEffect(() => {
		noticeRoster(roster);
	}, [roster]);

	useEffect(() => {
		setWindowTitle(selected?.persona.name ?? null);
	}, [selected]);

	useEffect(() => watchNotificationClicks(), []);

	// Opening a teammate is Ctrl+1 through Ctrl+9, in the rail's own order; the
	// rail says so on each row, because a shortcut nobody can see is no
	// shortcut. Ctrl+N adds one, Ctrl+, is settings, Ctrl+I is the teammate
	// on screen, Ctrl+F searches the conversation that is already on screen.
	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			if (!event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return;
			// By physical key as well as by character: a layout that puts
			// something else on the comma key still opens settings.
			if (event.key === "n" || event.code === "KeyN") {
				event.preventDefault();
				setSheet("new-teammate");
				return;
			}
			if (event.key === "," || event.code === "Comma") {
				event.preventDefault();
				setSheet("settings");
				return;
			}
			if (event.key === "i" || event.code === "KeyI") {
				if (selectedId === null) return;
				event.preventDefault();
				setSheet("teammate");
				return;
			}
			if (event.key === "f" || event.code === "KeyF") {
				if (sheet !== null || selectedId === null) return;
				event.preventDefault();
				setSearchOpen(true);
				return;
			}
			const seat = Number(event.key);
			if (!Number.isInteger(seat) || seat < 1 || seat > 9) return;
			const entry = roster[seat - 1];
			if (!entry) return;
			event.preventDefault();
			setSelectedId(entry.persona.id);
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [roster, selectedId, sheet]);

	/* A different teammate is a different conversation: the drawer was asking
	 * about the one that just left, so it closes rather than swapping its
	 * contents underneath a query you typed for someone else. */
	useEffect(() => {
		setSearchOpen(false);
	}, [selectedId]);

	return (
		<div className="flex h-full flex-col">
			{connection !== "open" && (
				<p role="status" className="border-b border-rule bg-paper-3 px-4 py-1 text-center text-xs text-ink-3">
					Reconnecting to Toad…
				</p>
			)}
			<div className="relative flex min-h-0 flex-1">
				<Rail
					entries={roster}
					selectedId={selectedId}
					seen={seen}
					onSelect={setSelectedId}
					onNew={() => setSheet("new-teammate")}
					onSettings={() => setSheet("settings")}
				/>

				<main className="flex min-w-0 flex-1 flex-col bg-paper">
					{selected ? (
						<Conversation
							key={selected.persona.id}
							entry={selected}
							roster={roster}
							models={models}
							searchOpen={searchOpen}
							focus={focus}
							onOpenTeammate={() => setSheet("teammate")}
							onOpenSearch={() => setSearchOpen((open) => !open)}
							onCloseSearch={() => setSearchOpen(false)}
							onPick={(personaId, eventId) => {
								setSearchOpen(false);
								setSelectedId(personaId);
								setFocus({ eventId, at: Date.now() });
							}}
						/>
					) : (
						<div className="flex flex-1 items-center justify-center px-6">
							<p className="max-w-sm text-center text-ink-3">
								Pick a teammate on the left, or add one.
							</p>
						</div>
					)}
				</main>

				{sheet === "new-teammate" && (
					<NewTeammate
						models={models}
						onCreated={(personaId) => {
							setSelectedId(personaId);
							setSheet(null);
						}}
						onClose={() => setSheet(null)}
					/>
				)}
				{sheet === "settings" && <Settings onClose={() => setSheet(null)} />}
				{sheet === "teammate" && selected && (
					<Teammate
						persona={selected.persona}
						onClose={() => setSheet(null)}
						onDeleted={() => {
							setSelectedId(null);
							setSheet(null);
						}}
					/>
				)}
			</div>
		</div>
	);
}

/**
 * One teammate's conversation. Keyed by teammate above, so switching tears the
 * tape subscription down and puts up another rather than folding two
 * conversations into one column.
 */
function Conversation({
	entry,
	roster,
	models,
	searchOpen,
	focus,
	onOpenTeammate,
	onOpenSearch,
	onCloseSearch,
	onPick,
}: {
	entry: RosterEntry;
	roster: RosterEntry[];
	models: ConfigChoice[];
	searchOpen: boolean;
	focus: { eventId: string; at: number } | null;
	onOpenTeammate(): void;
	onOpenSearch(): void;
	onCloseSearch(): void;
	onPick(personaId: string, eventId: string): void;
}) {
	const personaId = entry.persona.id;
	const { events, streaming } = useTape(personaId);
	const [replying, setReplying] = useState<ReplyTarget | null>(null);
	const [chapterSaid, setChapterSaid] = useState<string | null>(null);
	const [chapterBusy, setChapterBusy] = useState(false);

	const send = useCallback(
		(text: string) => {
			void wire.command("session.prompt", {
				personaId,
				text,
				...(replying ? { replyTo: replying.eventId } : {}),
			});
			setReplying(null);
		},
		[personaId, replying],
	);
	const start = useCallback(() => void wire.command("session.start", { personaId }), [personaId]);
	const cancel = useCallback(() => void wire.command("session.cancel", { personaId }), [personaId]);
	/* Success is the tape: the marker is superseded in place and the title
	 * lands on its line. Only a refusal needs a sentence here. */
	const startChapter = useCallback(() => {
		setChapterSaid(null);
		setChapterBusy(true);
		void wire
			.command("chapter.start_fresh", { personaId })
			.catch((error: Error) => setChapterSaid(error.message))
			.finally(() => setChapterBusy(false));
	}, [personaId]);

	// Escape clears a quote that is up even when the field is not focused.
	// The composer handles the same key first when the field has it, so a
	// turn is not cancelled on the same press.
	useEffect(() => {
		if (replying === null) return;
		const onKey = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			setReplying(null);
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [replying]);

	return (
		<>
			<ChatHeader
				entry={entry}
				models={models}
				searchOpen={searchOpen}
				chapterBusy={chapterBusy}
				chapterSaid={chapterSaid}
				onSetModel={(modelId) => void wire.command("session.set_model", { personaId, modelId })}
				onOpenTeammate={onOpenTeammate}
				onOpenSearch={onOpenSearch}
				onNewChapter={startChapter}
			/>
			<div className="relative flex min-h-0 flex-1 flex-col">
				<Transcript events={events} streaming={streaming} focus={focus} onReply={setReplying} />
				<Composer
					personaId={personaId}
					state={entry.session.state}
					replyQuote={replying?.text ?? null}
					onSend={send}
					onStart={start}
					onCancel={cancel}
					onClearReply={() => setReplying(null)}
				/>
				{searchOpen && (
					<SearchDrawer
						personaId={personaId}
						roster={roster}
						onClose={onCloseSearch}
						onPick={onPick}
					/>
				)}
			</div>
		</>
	);
}

/** Where this window last stood in each tape. Private mode or a full disk
 * just means every teammate looks unread until you open them again. */
const SEEN_KEY = "toad.rail.seen";

function loadSeen(): Record<string, number> {
	try {
		const raw = localStorage.getItem(SEEN_KEY);
		if (!raw) return {};
		const parsed: unknown = JSON.parse(raw);
		if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return {};
		const seen: Record<string, number> = {};
		for (const [id, ts] of Object.entries(parsed)) {
			if (typeof ts === "number" && Number.isFinite(ts)) seen[id] = ts;
		}
		return seen;
	} catch {
		return {};
	}
}

function saveSeen(seen: Record<string, number>): void {
	try {
		localStorage.setItem(SEEN_KEY, JSON.stringify(seen));
	} catch {
		// Quota, private mode — the next load treats everything as unread.
	}
}
