import { useCallback, useEffect, useState } from "react";
import type { Attachment, ConfigChoice } from "./generated/contract";
import { Chrome } from "./components/Chrome";
import { ChatHeader } from "./components/ChatHeader";
import { Composer } from "./components/Composer";
import { NewTeammate } from "./components/NewTeammate";
import { Rail } from "./components/Rail";
import { SearchDrawer } from "./components/SearchDrawer";
import { Settings, type SettingsSection } from "./components/Settings";
import { Transcript, type ReplyTarget } from "./components/Transcript";
import { confirmRemove, listenMenu } from "./native";
import { noticeRoster, setWindowTitle, watchNotificationClicks } from "./notify";
import { useTape } from "./tape";
import { wire, type Connection, type RosterEntry } from "./wire";

type Pane = "settings" | "new-teammate" | null;

export function App() {
	const [connection, setConnection] = useState<Connection>("connecting");
	const [roster, setRoster] = useState<RosterEntry[]>([]);
	const [seen, setSeen] = useState<Record<string, number>>(loadSeen);
	const [models, setModels] = useState<ConfigChoice[]>([]);
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const [pane, setPane] = useState<Pane>(null);
	const [settingsSection, setSettingsSection] = useState<SettingsSection>("general");
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
		if (settingsSection === "teammate" && selected === null) setSettingsSection("general");
	}, [settingsSection, selected]);

	useEffect(() => {
		noticeRoster(roster);
	}, [roster]);

	useEffect(() => {
		setWindowTitle(selected?.persona.name ?? null);
	}, [selected]);

	useEffect(() => watchNotificationClicks(), []);

	const closePane = useCallback(() => setPane(null), []);
	const openSettings = useCallback((section: SettingsSection = "general") => {
		setSearchOpen(false);
		setSettingsSection(section);
		setPane("settings");
	}, []);
	const openNew = useCallback(() => {
		setSearchOpen(false);
		setPane("new-teammate");
	}, []);
	const toggleSettings = useCallback(() => {
		setPane((current) => {
			if (current === "settings") return null;
			setSearchOpen(false);
			setSettingsSection((section) => (section === "teammate" ? "general" : section));
			return "settings";
		});
	}, []);
	const toggleTeammate = useCallback(() => {
		if (selectedId === null) return;
		setPane((current) => {
			if (current === "settings" && settingsSection === "teammate") return null;
			setSearchOpen(false);
			setSettingsSection("teammate");
			return "settings";
		});
	}, [selectedId, settingsSection]);
	const toggleNew = useCallback(() => {
		setPane((current) => {
			if (current === "new-teammate") return null;
			setSearchOpen(false);
			return "new-teammate";
		});
	}, []);

	const removeTeammate = useCallback(
		async (personaId: string, name: string) => {
			if (!(await confirmRemove(name))) return;
			try {
				await wire.command("persona.delete", { id: personaId });
				if (selectedId === personaId) {
					setSelectedId(null);
					setPane(null);
				}
			} catch {
				// The pane's own type-to-confirm is still there if this fails.
			}
		},
		[selectedId],
	);

	// Opening a teammate is Ctrl+1 through Ctrl+9, in the rail's own order; the
	// rail says so on each row, because a shortcut nobody can see is no
	// shortcut. Ctrl+N adds one, Ctrl+, is settings, Ctrl+I is the teammate
	// on screen, Ctrl+F searches the conversation that is already on screen.
	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			if (event.key === "Escape" && pane !== null) {
				event.preventDefault();
				setPane(null);
				return;
			}
			if (!event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return;
			// By physical key as well as by character: a layout that puts
			// something else on the comma key still opens settings.
			if (event.key === "n" || event.code === "KeyN") {
				event.preventDefault();
				if (takeChord()) toggleNew();
				return;
			}
			if (event.key === "," || event.code === "Comma") {
				event.preventDefault();
				if (takeChord()) toggleSettings();
				return;
			}
			if (event.key === "i" || event.code === "KeyI") {
				if (selectedId === null) return;
				event.preventDefault();
				if (takeChord()) toggleTeammate();
				return;
			}
			if (event.key === "f" || event.code === "KeyF") {
				if (pane !== null || selectedId === null) return;
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
			setPane(null);
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [roster, selectedId, pane, toggleNew, toggleSettings, toggleTeammate]);

	useEffect(() => {
		return listenMenu((id) => {
			if (id === "settings") {
				if (takeChord()) toggleSettings();
				return;
			}
			if (id === "new-teammate") {
				if (takeChord()) toggleNew();
				return;
			}
			if (id === "teammate") {
				if (takeChord()) toggleTeammate();
				return;
			}
			if (id === "search") {
				if (pane !== null || selectedId === null) return;
				setSearchOpen(true);
				return;
			}
			if (id.startsWith("teammate-")) {
				const seat = Number(id.slice("teammate-".length));
				const entry = roster[seat - 1];
				if (!entry) return;
				setSelectedId(entry.persona.id);
				setPane(null);
			}
		});
	}, [roster, selectedId, pane, toggleNew, toggleSettings, toggleTeammate]);

	/* A different teammate is a different conversation: the drawer was asking
	 * about the one that just left, so it closes rather than swapping its
	 * contents underneath a query you typed for someone else. */
	useEffect(() => {
		setSearchOpen(false);
	}, [selectedId]);

	/* Right-clicking chrome should not offer Reload. Fields and a live
	 * selection keep the system's own menu. A teammate row handles its own. */
	useEffect(() => {
		const suppress = (event: MouseEvent) => {
			const target = event.target as HTMLElement | null;
			if (target?.closest("input, textarea, [data-teammate-row]")) return;
			if (window.getSelection()?.isCollapsed === false) return;
			event.preventDefault();
		};
		document.addEventListener("contextmenu", suppress);
		return () => document.removeEventListener("contextmenu", suppress);
	}, []);

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
					onSelect={(id) => {
						setSelectedId(id);
						setPane(null);
					}}
					onNew={openNew}
					onSettings={() => openSettings("general")}
					onEdit={(id) => {
						setSelectedId(id);
						openSettings("teammate");
					}}
					onDelete={(id, name) => void removeTeammate(id, name)}
				/>

				<main className="flex min-w-0 flex-1 flex-col bg-paper">
					{pane === "settings" ? (
						<Settings
							section={settingsSection}
							onSection={setSettingsSection}
							teammate={selected?.persona ?? null}
							onClose={closePane}
							onDeleted={() => {
								setSelectedId(null);
								setPane(null);
							}}
						/>
					) : pane === "new-teammate" ? (
						<NewTeammate
							models={models}
							onCreated={(personaId) => {
								setSelectedId(personaId);
								setPane(null);
							}}
							onClose={closePane}
						/>
					) : selected ? (
						<Conversation
							key={selected.persona.id}
							entry={selected}
							roster={roster}
							models={models}
							searchOpen={searchOpen}
							focus={focus}
							onOpenTeammate={() => openSettings("teammate")}
							onOpenSearch={() => setSearchOpen((open) => !open)}
							onCloseSearch={() => setSearchOpen(false)}
							onPick={(personaId, eventId) => {
								setSearchOpen(false);
								setSelectedId(personaId);
								setFocus({ eventId, at: Date.now() });
							}}
						/>
					) : (
						<div className="flex min-h-0 flex-1 flex-col">
							<Chrome>
								<span className="text-ink-3" />
							</Chrome>
							<div className="flex flex-1 items-center justify-center px-6">
								<p className="max-w-sm text-center text-ink-3">
									Pick a teammate on the left, or add one.
								</p>
							</div>
						</div>
					)}
				</main>
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
		(text: string, attachments: Attachment[]) => {
			void wire.command("session.prompt", {
				personaId,
				text,
				...(replying ? { replyTo: replying.eventId } : {}),
				...(attachments.length > 0 ? { attachments } : {}),
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
	// Chips are put down first, on the window in capture, so this listener
	// does not also drop the quote on the same press. The composer handles
	// the key when the field has it, so a turn is not cancelled then either.
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
				onSetMode={(modeId) => void wire.command("session.set_mode", { personaId, modeId })}
				onOpenTeammate={onOpenTeammate}
				onOpenSearch={onOpenSearch}
				onNewChapter={startChapter}
			/>
			<div className="relative flex min-h-0 flex-1 flex-col">
				<Transcript
					personaId={personaId}
					events={events}
					streaming={streaming}
					focus={focus}
					onReply={setReplying}
				/>
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
/** The menu bar and the window both hear the same chord; one press is one
 * action, even when both fire. */
let lastChordAt = 0;
function takeChord(): boolean {
	const now = performance.now();
	if (now - lastChordAt < 120) return false;
	lastChordAt = now;
	return true;
}

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
