import { lazy, Suspense, useCallback, useEffect, useState } from "react";
import { useNarrow } from "./narrow";
import type { ConfigChoice } from "./generated/contract";
import { About } from "./components/About";
import { Conversation } from "./components/Conversation";
import { NewTeammate } from "./components/NewTeammate";
import { Rail, RAIL_FACES, RAIL_MIN, RailEdge, unreadOf, useRailSize } from "./components/Rail";
import { Titlebar } from "./ui/Titlebar";
import { WindowEdges } from "./ui/WindowEdges";
import type { SettingsSection } from "./components/Settings";
import { Teammate } from "./components/Teammate";
import { Shortcuts } from "./components/Shortcuts";
import { Subagent, type OpenSubagent } from "./components/Subagent";
import { Work, type OpenWork } from "./components/Work";
import { Thread, type OpenThread } from "./components/Thread";
import { Welcome } from "./components/Welcome";
import { matchChord } from "./chords";
import { confirmRemove, listenMenu, listenToastClicks, openLink, platform, setBadge, watchWindowShape } from "./native";
import { watchLooking } from "./looking";
import { noticeRoster, setWindowTitle, toastTarget } from "./notify";
import { useModelsRevision, useRoomJobs } from "./room";
import { Band } from "./ui/Band";
import { wire, type Connection, type RosterEntry } from "./wire";
import { activeDeskId, deskKey, LOCAL_DESK, setActiveDesk, useActiveDesk, useDesks } from "./desks";
import { syncWatches, useBackgroundUnread } from "./deskWatch";
import { AddDesk } from "./components/AddDesk";
import { ServerFiles } from "./components/ServerFiles";

/* Settings is opened now and then, not at launch: it loads on first open,
 * which keeps its nine sections out of the startup bundle. */
const Settings = lazy(() => import("./components/Settings").then((module) => ({ default: module.Settings })));
const SettingsRail = lazy(() => import("./components/Settings").then((module) => ({ default: module.SettingsRail })));


/** What stands in the conversation's place: a room-wide pane, or nothing. */
type Pane = "settings" | "new-teammate" | "shortcuts" | "about" | "add-desk" | null;

/** What can stand in the inspector's place beside a conversation. */
type Aside = { kind: "thread"; thread: OpenThread } | { kind: "subagent"; run: OpenSubagent } | { kind: "work"; work: OpenWork };

/**
 * The window for the active desk. Switching desks remounts all of it, so the
 * roster, the open teammate and every subscription start again on the other
 * desk and nothing keeps talking to the one left behind (see desks.ts).
 */
export function DeskRoot() {
	const desk = useActiveDesk();
	const desks = useDesks();
	// Every desk not on screen is watched for toasts and unread (deskWatch.ts).
	useEffect(syncWatches, [desk?.id, desks]);
	// Keyed by the endpoint too: a desk paired again has a new bridge, and its
	// old connection (and everything subscribed on it) is gone.
	return <App key={desk === null ? "none" : `${desk.id} ${desk.origin} ${desk.token}`} />;
}

export function App() {
	const [connection, setConnection] = useState<Connection>("connecting");
	/* The last roster this desk showed, so switching back to it draws at
	 * once while the fresh snapshot is on its way. */
	const [roster, setRoster] = useState<RosterEntry[]>(() => cachedRoster(activeDeskId() ?? "") ?? []);
	/* Whether the roster snapshot has landed. Before it, an empty roster is
	 * not an empty room, and the welcome pane would flash on every open. */
	const [rosterLoaded, setRosterLoaded] = useState(() => cachedRoster(activeDeskId() ?? "") !== undefined);
	useEffect(() => {
		if (rosterLoaded) keepRoster(activeDeskId() ?? "", roster);
	}, [roster, rosterLoaded]);
	const [seen, setSeen] = useState<Record<string, number>>(loadSeen);
	const [models, setModels] = useState<ConfigChoice[]>([]);
	const [selectedId, setSelectedId] = useState<string | null>(loadSelected);
	/* What stands in the inspector's place: a peer thread or a subagent's
	 * run, opened from its line in the conversation. */
	const [aside, setAside] = useState<Aside | null>(null);
	const [pane, setPane] = useState<Pane>(null);
	const [settingsSection, setSettingsSection] = useState<SettingsSection>("general");
	/* A narrow window keeps the pane and shows the rail as faces only, open
	 * or closed from the titlebar; there is no dragging it wider there. */
	const narrow = useNarrow();
	/* The rail: how wide you dragged it, whether it is down to faces, and whether you closed it. */
	const [railSize, setRailSize] = useRailSize();
	const toggleRail = useCallback(() => setRailSize((was) => ({ ...was, open: !was.open })), [setRailSize]);
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

	useEffect(() => watchLooking(), []);

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
		noticeRoster(roster, { id: activeDeskId() ?? "" });
	}, [roster]);

	// The dock's badge is the rail's unread count: the rows in bold, counted,
	// with those on the desks not on screen.
	const elsewhere = useBackgroundUnread();
	useEffect(() => {
		const others = Object.values(elsewhere).reduce((sum, count) => sum + count, 0);
		void setBadge(roster.filter((entry) => unreadOf(entry, selectedId, seen)).length + others);
	}, [roster, selectedId, seen, elsewhere]);

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
		setAside(null);
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
	}, []);

	// A clicked toast is a teammate asking to be looked at; the shell has
	// already raised the window.
	// One from a desk not on screen opens that desk on that teammate.
	useEffect(
		() =>
			listenToastClicks((payload) => {
				const { deskId, personaId } = toastTarget(payload);
				if (deskId === null || deskId === activeDeskId()) {
					select(personaId);
					return;
				}
				try {
					localStorage.setItem(deskKey(SELECTED_KEY, deskId), personaId);
				} catch {
					// Private mode: the desk opens where it was.
				}
				setActiveDesk(deskId);
			}),
		[select],
	);

	const closePane = useCallback(() => {
		setPane(null);
	}, []);
	const togglePane = useCallback(
		(id: Exclude<Pane, null>) => {
			setSearchOpen(false);
			if (pane === id) {
				closePane();
				return;
			}
			setPane(id);
		},
		[pane, closePane],
	);
	const toggleInspector = useCallback(
		(schedules = false) => {
			if (selectedId === null) return;
			setPane(null);
			setSearchOpen(false);
			setAside(null);
			setFocusSchedules(schedules);
			setInspector((open) => schedules || !open);
		},
		[selectedId],
	);
	const openAside = useCallback((next: Aside) => {
		setPane(null);
		setSearchOpen(false);
		setInspector(false);
		setAside(next);
	}, []);
	const openThread = useCallback((thread: OpenThread) => openAside({ kind: "thread", thread }), [openAside]);
	const openSubagent = useCallback((run: OpenSubagent) => openAside({ kind: "subagent", run }), [openAside]);
	/* A caption or the mark, pressed again with its work already open, closes it. */
	const openWork = useCallback(
		(work: OpenWork) => {
			if (aside?.kind === "work" && aside.work.personaId === work.personaId && aside.work.blockId === work.blockId) setAside(null);
			else openAside({ kind: "work", work });
		},
		[aside, openAside],
	);

	const removeTeammate = useCallback(
		async (personaId: string, name: string) => {
			if (!(await confirmRemove(name))) return;
			try {
				await wire.command("persona.delete", { id: personaId });
				if (selectedId === personaId) {
					setSelectedId(null);
					setInspector(false);
					setAside(null);
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
				if (aside !== null && !(event.target as HTMLElement | null)?.closest("textarea, input")) {
					event.preventDefault();
					setAside(null);
					return;
				}
				if (inspector && !(event.target as HTMLElement | null)?.closest("textarea, input")) {
					event.preventDefault();
					setInspector(false);
					return;
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
			if (chord === "sidebar") {
				event.preventDefault();
				if (takeChord()) toggleRail();
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
	}, [roster, selectedId, pane, inspector, aside, searchOpen, select, closePane, togglePane, toggleInspector, toggleRail]);

	useEffect(() => {
		return listenMenu((id) => {
			if (id === "sidebar") {
				if (takeChord()) toggleRail();
				return;
			}
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
	}, [roster, selectedId, pane, select, togglePane, toggleInspector, toggleRail]);

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

	const welcome = rosterLoaded && roster.length === 0;
	/* A narrow window has room for faces beside the pane and no more. */
	const faces = narrow || railSize.compact;
	/* Settings' sections have no faces to fall back to: they stand at the
	 * names' width, and at the narrowest of it in a narrow window. */
	const settingsWidth = narrow ? RAIL_MIN : railSize.width;

	return (
		<div className="flex h-full flex-col">
			<Titlebar
				searchable={pane === null && selected !== null}
				searchOpen={searchOpen}
				onToggleSearch={() => setSearchOpen((open) => !open)}
				rail={{ open: railSize.open, onToggle: toggleRail }}
			/>
			{platform() === "linux" && <WindowEdges />}
			<ServerFiles />
			<div className="flex min-h-0 flex-1 gap-2 p-2 pt-0">
			{!railSize.open ? null : pane === "settings" ? (
				<Suspense fallback={null}>
					<SettingsRail section={settingsSection} onSection={setSettingsSection} onBack={closePane} width={settingsWidth} />
				</Suspense>
			) : (
			<Rail
				entries={roster}
				loaded={rosterLoaded}
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
				width={faces ? RAIL_FACES : railSize.width}
				compact={faces}
			/>
			)}
			{!narrow && <RailEdge size={railSize} onSize={setRailSize} />}

			<main className="@container flex min-w-0 flex-1 flex-col gap-0">
				<DeskBand onAddDesk={() => togglePane("add-desk")} />
				<div className="flex min-h-0 min-w-0 flex-1 gap-2">
				{pane === "settings" ? (
					<Suspense fallback={null}>
						<Settings section={settingsSection} onAddDesk={() => togglePane("add-desk")} />
					</Suspense>
				) : pane === "shortcuts" ? (
					<Shortcuts onClose={closePane} />
				) : pane === "about" ? (
					<About onClose={closePane} />
				) : pane === "add-desk" ? (
					<AddDesk onClose={closePane} />
				) : pane === "new-teammate" ? (
					<NewTeammate models={models} onCreated={select} onClose={closePane} />
				) : welcome ? (
					<Welcome models={models} onCreated={select} />
				) : selected ? (
					<>
						<Conversation
							key={selected.persona.id}
							entry={selected}
							models={models}
							onSaid={setSaid}
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
							onOpenSubagent={openSubagent}
							onOpenWork={(blockId) => openWork({ personaId: selected.persona.id, blockId })}
							workOpen={aside?.kind === "work" ? aside.work.blockId : undefined}
						/>
						{aside?.kind === "thread" ? (
							<Thread
								key={`thread-${aside.thread.key}`}
								open={aside.thread}
								selfId={selected.persona.id}
								selfName={selected.persona.name}
								onClose={() => setAside(null)}
							/>
						) : aside?.kind === "work" ? (
							<Work
								key={`work-${aside.work.personaId}`}
								open={aside.work}
								name={selected.persona.name}
								live={selected.session.state === "thinking"}
								onClose={() => setAside(null)}
							/>
						) : aside?.kind === "subagent" ? (
							<Subagent
								key={`run-${aside.run.runId}`}
								open={aside.run}
								selfId={selected.persona.id}
								selfName={selected.persona.name}
								onClose={() => setAside(null)}
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
				</div>
			</main>
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

/** Each desk's last roster, for the moment after switching back to it. */
const rosterCache = new Map<string, RosterEntry[]>();
const ROSTER_KEY = "hotline.desk.roster";

/**
 * A remote desk's roster is also kept across restarts: a desk that can't be
 * reached at launch still shows its teammates as they last were, which is
 * what the "Can't reach" band promises. The local desk is always there.
 */
function cachedRoster(deskId: string): RosterEntry[] | undefined {
	const held = rosterCache.get(deskId);
	if (held !== undefined || deskId === LOCAL_DESK) return held;
	try {
		const raw = localStorage.getItem(`${ROSTER_KEY}:${deskId}`);
		const kept = raw === null ? undefined : (JSON.parse(raw) as RosterEntry[]);
		if (Array.isArray(kept)) rosterCache.set(deskId, kept);
		return Array.isArray(kept) ? kept : undefined;
	} catch {
		return undefined;
	}
}

function keepRoster(deskId: string, roster: RosterEntry[]) {
	rosterCache.set(deskId, roster);
	if (deskId === LOCAL_DESK) return;
	try {
		localStorage.setItem(`${ROSTER_KEY}:${deskId}`, JSON.stringify(roster));
	} catch {
		// Quota, private mode: the in-memory copy still serves this session.
	}
}

/** The open teammate survives a reload, which is what makes the tape
 * subscribe able to race wire.connect() — see watchWhenOpen in tape.ts. */
const SELECTED_KEY = "hotline.rail.selected";

function perDesk(key: string): string {
	return deskKey(key);
}

function loadSelected(): string | null {
	try {
		const id = localStorage.getItem(perDesk(SELECTED_KEY));
		return id !== null && id !== "" ? id : null;
	} catch {
		return null;
	}
}

function saveSelected(id: string | null): void {
	try {
		if (id === null) localStorage.removeItem(perDesk(SELECTED_KEY));
		else localStorage.setItem(perDesk(SELECTED_KEY), id);
	} catch {
		// Quota, private mode.
	}
}

/** Where this window last stood in each tape. Private mode or a full disk
 * just means every teammate looks unread until you open them again. */
const SEEN_KEY = "hotline.rail.seen";

function loadSeen(): Record<string, number> {
	try {
		const raw = localStorage.getItem(perDesk(SEEN_KEY));
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
		localStorage.setItem(perDesk(SEEN_KEY), JSON.stringify(seen));
	} catch {
		// Quota, private mode — the next load treats everything as unread.
	}
}

/**
 * A band over the active desk when it is a remote one that is not there:
 * unreachable (the bridge keeps trying; what shows is what it last said),
 * or no longer paired (the server revoked this computer). Nothing for a
 * local desk or a remote one that is open.
 */
function DeskBand({ onAddDesk }: { onAddDesk(): void }) {
	const desk = useActiveDesk();
	if (desk === null || desk.kind !== "remote" || desk.state === undefined || desk.state === "open") return null;
	const revoked = desk.state === "revoked";
	return (
		<p role="status" className="flex shrink-0 items-center gap-2 rounded-md bg-raised px-4 py-1.5 text-sm text-ink">
			<span aria-hidden="true" className={`h-1.5 w-1.5 shrink-0 rounded-full ${revoked ? "" : "beat"}`} style={{ background: "var(--warn)" }} />
			<span className="min-w-0 flex-1">
				{revoked
					? `${desk.name} no longer recognises this computer. Pair it again to reach it.`
					: desk.state === "connecting"
						? `Connecting to ${desk.name}…`
						: `Can't reach ${desk.name}${desk.error ? ` (${desk.error})` : ""}. Hotline keeps trying; what you see is what it last said.`}
			</span>
			{revoked && (
				<button type="button" className="control btn btn-sm" onClick={onAddDesk}>
					Pair again
				</button>
			)}
		</p>
	);
}
