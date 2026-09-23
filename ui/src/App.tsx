import { useCallback, useEffect, useState } from "react";
import { useNarrow } from "./narrow";
import type { ConfigChoice } from "./generated/contract";
import { About } from "./components/About";
import { Conversation } from "./components/Conversation";
import { NewTeammate } from "./components/NewTeammate";
import { Rail, unreadOf } from "./components/Rail";
import { Titlebar } from "./ui/Titlebar";
import { Settings, SettingsRail, type SettingsSection } from "./components/Settings";
import { Shortcuts } from "./components/Shortcuts";
import { Teammate } from "./components/Teammate";
import { Thread, type OpenThread } from "./components/Thread";
import { Welcome } from "./components/Welcome";
import { matchChord } from "./chords";
import { confirmRemove, listenMenu, listenToastClicks, openLink, platform, setBadge, watchWindowShape } from "./native";
import { noticeRoster, setWindowTitle } from "./notify";
import { useModelsRevision, useRoomJobs } from "./room";
import { Band } from "./ui/Band";
import { wire, type Connection, type RosterEntry } from "./wire";

/** What stands in the conversation's place: a room-wide pane, or nothing. */
type Pane = "settings" | "new-teammate" | "shortcuts" | "about" | null;

export function App() {
	const [connection, setConnection] = useState<Connection>("connecting");
	const [roster, setRoster] = useState<RosterEntry[]>([]);
	/* Whether the roster snapshot has landed. Before it, an empty roster is
	 * not an empty room, and the welcome pane would flash on every open. */
	const [rosterLoaded, setRosterLoaded] = useState(false);
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
	/* What the strip's model or effort picker was refused with. The strip
	 * has no room for a sentence, so the conversation says it; a new
	 * teammate is a new conversation, so it goes when the selection does. */
	const [said, setSaid] = useState<string | null>(null);
	useEffect(() => {
		setSaid(null);
	}, [selectedId]);
	const [focus, setFocus] = useState<{ eventId: string; at: number } | null>(null);
	const jobs = useRoomJobs();
	const modelsRevision = useModelsRevision();

	useEffect(() => {
		wire.connect();
		return wire.onConnection(setConnection);
	}, []);

	useEffect(() => {
		return wire.subscribe<RosterEntry>(
			{ view: "roster" },
			{
				snapshot: (entries) => {
					setRoster(entries);
					setRosterLoaded(true);
				},
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
	 * reconnect, and whenever the room says they changed: a key added on
	 * another seat, a provider's list refreshed, or a filter saved here. */
	useEffect(() => {
		if (connection !== "open") return;
		let current = true;
		wire
			.command("models.list", {})
			.then((next) => current && setModels(next))
			.catch(() => current && setModels([]));
		return () => {
			current = false;
		};
	}, [connection, modelsRevision]);

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

	// The dock's badge is the rail's unread count: the rows in bold, counted.
	useEffect(() => {
		void setBadge(roster.filter((entry) => unreadOf(entry, selectedId, seen)).length);
	}, [roster, selectedId, seen]);

	// In native fullscreen the traffic lights leave with the menu bar, and
	// the rail's gutter for them goes too (index.css).
	useEffect(() => {
		if (platform() !== "macos") return;
		return watchWindowShape(({ fullscreen }) => {
			document.documentElement.toggleAttribute("data-fullscreen", fullscreen);
		});
	}, []);

	useEffect(() => {
		setWindowTitle(selected?.persona.name ?? null);
	}, [selected]);

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

	// A clicked toast is a teammate asking to be looked at; the shell has
	// already raised the window.
	useEffect(() => listenToastClicks(select), [select]);

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
				// The inspector's own Remove reports a refusal if this fails.
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
				void openLink("https://github.com/1broseidon/hotline");
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
	 * back key in its band. An empty room's welcome pane counts as something
	 * to look at. Wide: both, and no back key. */
	const welcome = rosterLoaded && roster.length === 0;
	const railOnly = narrow && (railShown || (pane === null && selected === null && !welcome));
	const back = narrow ? () => setRailShown(true) : undefined;

	return (
		<div className="flex h-full flex-col">
			<Titlebar
				selected={selected}
				models={models}
				searchable={pane === null && selected !== null}
				searchOpen={searchOpen}
				onToggleSearch={() => setSearchOpen((open) => !open)}
				onSaid={setSaid}
			/>
			<div className="flex min-h-0 flex-1 gap-2 p-2 pt-0">
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
					if (id === "github") void openLink("https://github.com/1broseidon/hotline");
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
				) : welcome ? (
					<Welcome models={models} onCreated={select} />
				) : selected ? (
					<>
						<Conversation
							key={selected.persona.id}
							entry={selected}
							{...(back !== undefined ? { onBack: back } : {})}
							roster={roster}
							jobs={jobs.filter((job) => job.personaId === selected.persona.id)}
							said={said}
							searchOpen={searchOpen}
							inspectorOpen={inspector}
							focus={focus}
							onToggleInspector={() => toggleInspector()}
							onOpenSchedules={() => toggleInspector(true)}
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
									session={selected.session}
									jobs={jobs.filter((job) => job.personaId === selected.persona.id)}
									roster={roster}
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
							<p className="text-center text-ink-3">{rosterLoaded ? "Pick a teammate on the left." : ""}</p>
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
const SELECTED_KEY = "hotline.rail.selected";

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
const SEEN_KEY = "hotline.rail.seen";

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
