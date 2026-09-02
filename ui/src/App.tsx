import { useCallback, useEffect, useState } from "react";
import { useNarrow } from "./narrow";
import type { ConfigChoice } from "./generated/contract";
import { About } from "./components/About";
import { Conversation } from "./components/Conversation";
import { NewTeammate } from "./components/NewTeammate";
import { Rail } from "./components/Rail";
import { Titlebar } from "./ui/Titlebar";
import { Settings, SettingsRail, type SettingsSection } from "./components/Settings";
import { Shortcuts } from "./components/Shortcuts";
import { Teammate } from "./components/Teammate";
import { Thread, type OpenThread } from "./components/Thread";
import { matchChord } from "./chords";
import { PlusIcon } from "./icons";
import { confirmRemove, drawsFrame, listenMenu, openLink } from "./native";
import { noticeRoster, setWindowTitle, watchNotificationClicks } from "./notify";
import { useRoomJobs, useRoomSettings } from "./room";
import { Band } from "./ui/Band";
import { wire, type Connection, type RosterEntry } from "./wire";

/** What stands in the conversation's place: a room-wide pane, or nothing. */
type Pane = "settings" | "new-teammate" | "shortcuts" | "about" | null;

export function App() {
	const [connection, setConnection] = useState<Connection>("connecting");
	const [roster, setRoster] = useState<RosterEntry[]>([]);
	const [seen, setSeen] = useState<Record<string, number>>(loadSeen);
	const [models, setModels] = useState<ConfigChoice[]>([]);
	const [selectedId, setSelectedId] = useState<string | null>(loadSelected);
	const [thread, setThread] = useState<OpenThread | null>(null);
	const [pane, setPane] = useState<Pane>(null);
	const [settingsSection, setSettingsSection] = useState<SettingsSection>("general");
	/* A narrow window shows one thing at a time, the way a phone does: the
	 * rail, or what was chosen in it. This is which, and it means nothing
	 * once the window is wide enough for both. */
	const narrow = useNarrow();
	const [railShown, setRailShown] = useState(false);
	/* The teammate's own pane sits beside the conversation, not in its place:
	 * you edit a colleague while watching them work. */
	const [inspector, setInspector] = useState(false);
	const [focusSchedules, setFocusSchedules] = useState(false);
	const [searchOpen, setSearchOpen] = useState(false);
	const [focus, setFocus] = useState<{ eventId: string; at: number } | null>(null);
	const jobs = useRoomJobs();
	const { enabledModels } = useRoomSettings();
	const enabledKey = JSON.stringify(enabledModels);

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

	/* The room's models are asked for once a socket is up, again after a
	 * reconnect, and when the saved filter changes: a key added on another
	 * seat, or a filter saved here, changes the answer. */
	useEffect(() => {
		if (connection !== "open") return;
		wire
			.command("models.list", {})
			.then(setModels)
			.catch(() => setModels([]));
	}, [connection, enabledKey]);

	const selected = roster.find((one) => one.persona.id === selectedId) ?? null;

	/* Opening a teammate, or sitting on one while a new line lands, is what
	 * "shown" means. The rail then has a ts to compare against. */
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
		noticeRoster(roster);
	}, [roster]);

	useEffect(() => {
		setWindowTitle(selected?.persona.name ?? null);
	}, [selected]);

	useEffect(() => watchNotificationClicks(), []);

	/* A different teammate is a different conversation: the search was asking
	 * about the one that just left, and the inspector was editing them. */
	useEffect(() => {
		setSearchOpen(false);
		setFocusSchedules(false);
		setThread(null);
		saveSelected(selectedId);
	}, [selectedId]);

	useEffect(() => {
		if (selectedId !== null && roster.length > 0 && !roster.some((one) => one.persona.id === selectedId)) {
			setSelectedId(null);
		}
	}, [roster, selectedId]);

	const select = useCallback((personaId: string) => {
		setSelectedId(personaId);
		setPane(null);
		setRailShown(false);
	}, []);
	/* Closing a pane lands on the rail: it is where the pane was opened from. */
	const closePane = useCallback(() => {
		setPane(null);
		setRailShown(true);
	}, []);
	const togglePane = useCallback(
		(id: Exclude<Pane, null>) => {
			setSearchOpen(false);
			if (pane === id) {
				closePane();
				return;
			}
			setPane(id);
			// Settings opens on its menu, which is the rail; the rest are the pane itself.
			setRailShown(id === "settings");
		},
		[pane, closePane],
	);
	const toggleInspector = useCallback(
		(schedules = false) => {
			if (selectedId === null) return;
			setPane(null);
			setSearchOpen(false);
			setThread(null);
			setFocusSchedules(schedules);
			setInspector((open) => schedules || !open);
		},
		[selectedId],
	);
	const openThread = useCallback((next: OpenThread) => {
		setPane(null);
		setSearchOpen(false);
		setInspector(false);
		setThread(next);
	}, []);

	const removeTeammate = useCallback(
		async (personaId: string, name: string) => {
			if (!(await confirmRemove(name))) return;
			try {
				await wire.command("persona.delete", { id: personaId });
				if (selectedId === personaId) {
					setSelectedId(null);
					setInspector(false);
					setThread(null);
				}
			} catch {
				// The inspector's own type-to-confirm is still there if this fails.
			}
		},
		[selectedId],
	);

	// Chords live in chords.ts so Help cannot list a key the window does
	// not hear. Escape closes whatever is on top: the search, then a pane,
	// then the inspector.
	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			const chord = matchChord(event);
			if (chord === "close") {
				if (searchOpen) return; // the search closes itself
				if (pane !== null) {
					event.preventDefault();
					closePane();
					return;
				}
				if (thread !== null && !(event.target as HTMLElement | null)?.closest("textarea, input")) {
					event.preventDefault();
					setThread(null);
					return;
				}
				if (inspector && !(event.target as HTMLElement | null)?.closest("textarea, input")) {
					event.preventDefault();
					setInspector(false);
					return;
				}
				// With nothing else on top, a narrow window's Escape is the back key.
				if (narrow && !railShown && !(event.target as HTMLElement | null)?.closest("textarea, input")) {
					event.preventDefault();
					setRailShown(true);
				}
				return;
			}
			if (chord === "new-teammate") {
				event.preventDefault();
				if (takeChord()) togglePane("new-teammate");
				return;
			}
			if (chord === "settings") {
				event.preventDefault();
				if (takeChord()) togglePane("settings");
				return;
			}
			if (chord === "teammate") {
				if (selectedId === null) return;
				event.preventDefault();
				if (takeChord()) toggleInspector();
				return;
			}
			if (chord === "search") {
				if (pane !== null || selectedId === null) return;
				event.preventDefault();
				setSearchOpen(true);
				return;
			}
			if (chord === null || !chord.startsWith("teammate-")) return;
			const seat = Number(chord.slice("teammate-".length));
			const entry = roster[seat - 1];
			if (!entry) return;
			event.preventDefault();
			select(entry.persona.id);
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [roster, selectedId, pane, inspector, thread, searchOpen, narrow, railShown, select, closePane, togglePane, toggleInspector]);

	useEffect(() => {
		return listenMenu((id) => {
			if (id === "settings") {
				if (takeChord()) togglePane("settings");
				return;
			}
			if (id === "new-teammate") {
				if (takeChord()) togglePane("new-teammate");
				return;
			}
			if (id === "teammate") {
				if (takeChord()) toggleInspector();
				return;
			}
			if (id === "search") {
				if (pane !== null || selectedId === null) return;
				setSearchOpen(true);
				return;
			}
			if (id === "about") {
				togglePane("about");
				return;
			}
			if (id === "shortcuts") {
				togglePane("shortcuts");
				return;
			}
			if (id === "github") {
				void openLink("https://github.com/1broseidon/toad");
				return;
			}
			if (id.startsWith("teammate-")) {
				const seat = Number(id.slice("teammate-".length));
				const entry = roster[seat - 1];
				if (entry) select(entry.persona.id);
			}
		});
	}, [roster, selectedId, pane, select, togglePane, toggleInspector]);

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

	/* Narrow: the rail alone when it is what you are looking at, or when
	 * there is nothing else to look at; otherwise the pane alone, with a
	 * back key in its band. Wide: both, and no back key. */
	const railOnly = narrow && (railShown || (pane === null && selected === null));
	const back = narrow ? () => setRailShown(true) : undefined;

	return (
		<div className="flex h-full flex-col">
			{drawsFrame() && <Titlebar name={selected?.persona.name ?? null} />}
			<div className={drawsFrame() ? "flex min-h-0 flex-1 gap-2 p-2 pt-0" : "flex min-h-0 flex-1 gap-2 p-2"}>
			{narrow && !railOnly ? null : pane === "settings" ? (
				<SettingsRail
					section={settingsSection}
					onSection={(section) => {
						setSettingsSection(section);
						setRailShown(false);
					}}
					onBack={closePane}
				/>
			) : (
			<Rail
				entries={roster}
				selectedId={selectedId}
				seen={seen}
				connection={connection}
				onSelect={select}
				onNew={() => togglePane("new-teammate")}
				onSettings={() => togglePane("settings")}
				onEdit={(id) => {
					select(id);
					setInspector(true);
				}}
				onDelete={(id, name) => void removeTeammate(id, name)}
				onHelp={(id) => {
					if (id === "github") void openLink("https://github.com/1broseidon/toad");
					else togglePane(id);
				}}
			/>
			)}

			{railOnly ? null : (
			<main className="flex min-w-0 flex-1 gap-2">
				{pane === "settings" ? (
					<Settings section={settingsSection} {...(back !== undefined ? { onBack: back } : {})} />
				) : pane === "shortcuts" ? (
					<Shortcuts onClose={closePane} />
				) : pane === "about" ? (
					<About onClose={closePane} />
				) : pane === "new-teammate" ? (
					<NewTeammate models={models} onCreated={select} onClose={closePane} />
				) : selected ? (
					<>
						<Conversation
							key={selected.persona.id}
							entry={selected}
							{...(back !== undefined ? { onBack: back } : {})}
							roster={roster}
							models={models}
							jobs={jobs.filter((job) => job.personaId === selected.persona.id)}
							searchOpen={searchOpen}
							inspectorOpen={inspector}
							focus={focus}
							onToggleInspector={() => toggleInspector()}
							onOpenSchedules={() => toggleInspector(true)}
							onToggleSearch={() => setSearchOpen((open) => !open)}
							onCloseSearch={() => setSearchOpen(false)}
							onDelete={() => void removeTeammate(selected.persona.id, selected.persona.name)}
							onPick={(personaId, eventId) => {
								setSearchOpen(false);
								setSelectedId(personaId);
								setFocus({ eventId, at: Date.now() });
							}}
							onOpenThread={openThread}
						/>
						{thread !== null ? (
							<Thread
								key={`thread-${thread.key}`}
								open={thread}
								selfId={selected.persona.id}
								selfName={selected.persona.name}
								onClose={() => setThread(null)}
							/>
						) : (
							inspector && (
								<Teammate
									key={`inspector-${selected.persona.id}`}
									persona={selected.persona}
									jobs={jobs.filter((job) => job.personaId === selected.persona.id)}
									focusSchedules={focusSchedules}
									onClose={() => setInspector(false)}
									onDeleted={() => {
										setSelectedId(null);
										setInspector(false);
									}}
									onOpenThread={openThread}
								/>
							)
						)}
					</>
				) : (
					<div className="pane">
						<Band>
							<span />
						</Band>
						<div className="flex flex-1 flex-col items-center justify-center gap-4 px-6 pb-10">
							<p className="text-center text-ink-3">
								{roster.length === 0
									? "Add a teammate to open the room."
									: "Pick a teammate on the left."}
							</p>
							{roster.length === 0 && (
								<button type="button" className="control btn" onClick={() => togglePane("new-teammate")}>
									<PlusIcon />
									New teammate
								</button>
							)}
						</div>
					</div>
				)}
			</main>
			)}
			</div>
		</div>
	);
}

/** The menu bar and the window both hear the same chord; one press is one
 * action, even when both fire. */
let lastChordAt = 0;
function takeChord(): boolean {
	const now = performance.now();
	if (now - lastChordAt < 120) return false;
	lastChordAt = now;
	return true;
}

/** The open teammate survives a reload, which is what makes the tape
 * subscribe able to race wire.connect() — see watchWhenOpen in tape.ts. */
const SELECTED_KEY = "toad.rail.selected";

function loadSelected(): string | null {
	try {
		const id = localStorage.getItem(SELECTED_KEY);
		return id !== null && id !== "" ? id : null;
	} catch {
		return null;
	}
}

function saveSelected(id: string | null): void {
	try {
		if (id === null) localStorage.removeItem(SELECTED_KEY);
		else localStorage.setItem(SELECTED_KEY, id);
	} catch {
		// Quota, private mode.
	}
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
