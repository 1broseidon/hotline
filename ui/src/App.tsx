import { lazy, Suspense, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useNarrow } from "./narrow";
import { shownState } from "./activity";
import type { ConfigChoice } from "./generated/contract";
import { Conversation } from "./components/Conversation";
import { NewTeammate } from "./components/NewTeammate";
import { Rail, RAIL_FACES, RAIL_MIN, RailEdge, unreadOf, useRailSize } from "./components/Rail";
import { Dock, type DockState } from "./components/Dock";
import { clampDock, dockOverlays, loadDockWidth, saveDockWidth } from "./dock";
import { Titlebar } from "./ui/Titlebar";
import { CallBar, DeskCallCard, DeskCallKey } from "./components/Call";
import { TeamSearch } from "./components/Search";
import { UpdateFloat } from "./components/UpdateFloat";
import { closeCall, startCall, useCall, useCallSnapshot, useVoiceSupport } from "./voice/call";
import { WindowEdges } from "./ui/WindowEdges";
import { Teammate } from "./components/Teammate";
import { ProcessWork, sameWork, Work, type OpenWork } from "./components/Work";
import type { ThreadRef } from "./components/Transcript";
import { EmptyRoom, Welcome } from "./components/Welcome";
import { LeadPrompt } from "./components/LeadPrompt";
import { confirmRemove, listenToastClicks, platform, setBadge, watchWindowShape } from "./native";
import { watchLooking } from "./looking";
import { noticeRoster, setWindowTitle, toastTarget } from "./notify";
import { useModelsRevision, useRoomJobs } from "./room";
import { Band } from "./ui/Band";
import { wire, type Connection, type RosterEntry } from "./wire";
import { activeDeskId, useActiveDesk, useDesks, wireFor } from "./desks";
import { cachedRoster, keepRoster, syncWatches, useBackgroundRosters, useBackgroundUnread } from "./deskWatch";
import { AddDesk } from "./components/AddDesk";
import { ServerFiles } from "./components/ServerFiles";
import { useSaved } from "./useSaved";
import { openOnDesk, useNavigation } from "./useNavigation";
import { useSeen } from "./useSeen";
import { useWindowHotkeys } from "./useWindowHotkeys";


/* Settings is opened now and then, not at launch: it loads on first open,
 * which keeps its nine sections out of the startup bundle. */
const Settings = lazy(() => import("./components/Settings").then((module) => ({ default: module.Settings })));
const SettingsRail = lazy(() => import("./components/Settings").then((module) => ({ default: module.SettingsRail })));

/**
 * The window for the active desk: the desk of the teammate on screen.
 * Opening a teammate on another desk remounts all of it, so the open
 * teammate and every subscription start again on that desk and nothing
 * keeps talking to the one left behind (see desks.ts). The rail draws every
 * desk's teammates either way, from what each desk last said.
 */
export function DeskRoot() {
	const desk = useActiveDesk();
	const desks = useDesks();
	// Every desk not on screen is watched for toasts and unread (deskWatch.ts).
	// biome-ignore lint/correctness/useExhaustiveDependencies: syncWatches reads desks.ts itself; these are what change it.
	useEffect(syncWatches, [desk?.id, desks]);
	// Keyed by the endpoint too: a desk paired again has a new bridge, and its
	// old connection (and everything subscribed on it) is gone.
	return <App key={desk === null ? "none" : `${desk.id} ${desk.origin} ${desk.token}`} />;
}

export function App() {
	const desk = useActiveDesk();
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
	const [models, setModels] = useState<ConfigChoice[]>([]);
	const {
		selectedId,
		setSelectedId,
		setSpaces,
		dock,
		setDock,
		inspector,
		setInspector,
		pane,
		setPane,
		settingsSection,
		setSettingsSection,
		select,
		closePane,
		togglePane,
		newTeammate,
	} = useNavigation();
	/* The work card each teammate has open, by persona: it belongs to them,
	 * so it goes when you leave them and is there again when you come back. */
	const [works, setWorks] = useState<Record<string, OpenWork>>({});
	const workOf = selectedId !== null && selectedId in works ? works[selectedId]! : undefined;
	const closeWork = useCallback((personaId: string) => {
		setWorks((was) => {
			const { [personaId]: _, ...rest } = was;
			return rest;
		});
	}, []);
	/* The right-hand pane: this teammate's threads, beside the
	 * conversation or over it in a window too narrow. */
	const [dockWidth, setDockWidthState] = useState(loadDockWidth);
	const setDockWidth = useCallback((width: number) => setDockWidthState(clampDock(width)), []);
	useSaved(dockWidth, saveDockWidth);
	/* Whether the pane would leave the conversation too narrow to read beside
	 * it. Only this fact is kept, not the width: a drag of the window edge
	 * reports a new width every frame, and a number would draw the whole app
	 * for each of them. */
	const [paneOverlays, setPaneOverlays] = useState(false);
	/* A narrow window keeps the pane and shows the rail as faces only, open
	 * or closed from the titlebar; there is no dragging it wider there. */
	const narrow = useNarrow();
	/* Under this the floating work card would sit on the conversation's
	 * words, so it docks under the composer instead. */
	const mainRef = useRef<HTMLElement>(null);
	const [dockWork, setDockWork] = useState(false);
	const dockWidthRef = useRef(dockWidth);
	dockWidthRef.current = dockWidth;
	const measured = useRef<() => void>(() => {});
	useLayoutEffect(() => {
		const el = mainRef.current;
		if (el === null) return;
		const measure = () => {
			setDockWork(el.clientWidth < WORK_DOCK_BELOW);
			setPaneOverlays(dockOverlays(el.clientWidth, dockWidthRef.current));
		};
		measured.current = measure;
		const observer = new ResizeObserver(measure);
		observer.observe(el);
		return () => observer.disconnect();
	}, []);
	// The pane's own width is the other half of the sum.
	// biome-ignore lint/correctness/useExhaustiveDependencies: the measure reads the width through dockWidthRef; this re-runs it.
	useLayoutEffect(() => measured.current(), [dockWidth]);
	/* The rail: how wide you dragged it, whether it is down to faces, and whether you closed it. */
	const [railSize, setRailSize] = useRailSize();
	const toggleRail = useCallback(() => setRailSize((was) => ({ ...was, open: !was.open })), [setRailSize]);
	/* The teammate's own pane sits beside the conversation, not in its place:
	 * you edit a colleague while watching them work. */
	const [focusSchedules, setFocusSchedules] = useState(false);
	/* The Search chord, as a count: each press asks the team's search for the caret. */
	const [searchAsk, setSearchAsk] = useState(0);
	/* What the strip's model or effort picker was refused with. The strip
	 * has no room for a sentence, so the conversation says it; a new
	 * teammate is a new conversation, so it goes when the selection does. */
	const [said, setSaid] = useState<string | null>(null);
	// biome-ignore lint/correctness/useExhaustiveDependencies: a change of teammate is what clears it.
	useEffect(() => {
		setSaid(null);
	}, [selectedId]);
	const [focus, setFocus] = useState<{ eventId: string; at: number } | null>(null);
	const jobs = useRoomJobs();
	const modelsRevision = useModelsRevision();
	/* The other desks' teammates, for the one rail and its ⌃1–9. */
	const elsewhere = useBackgroundRosters();

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
	// biome-ignore lint/correctness/useExhaustiveDependencies: modelsRevision is the room saying the list changed.
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

	const seen = useSeen(selectedId, selected?.latest);

	useEffect(() => {
		noticeRoster(roster, { id: activeDeskId() ?? "" });
	}, [roster]);

	// The dock's badge is the rail's unread count: the rows in bold, counted,
	// with those on the desks not on screen.
	const unreadElsewhere = useBackgroundUnread();
	const badge = useMemo(
		() => roster.filter((entry) => unreadOf(entry, selectedId, seen)).length + Object.values(unreadElsewhere).reduce((sum, count) => sum + count, 0),
		[roster, selectedId, seen, unreadElsewhere],
	);
	useEffect(() => {
		void setBadge(badge);
	}, [badge]);

	// In native fullscreen the traffic lights leave with the menu bar, and
	// the rail's gutter for them goes too (index.css).
	useEffect(() => {
		if (platform() !== "macos") return;
		return watchWindowShape(({ fullscreen }) => {
			document.documentElement.toggleAttribute("data-fullscreen", fullscreen);
		});
	}, []);

	const name = selected?.persona.name ?? null;
	useEffect(() => {
		setWindowTitle(name);
	}, [name]);

	/* A different teammate is a different conversation: the search was asking
	 * about the one that just left, and the inspector was editing them. */
	// biome-ignore lint/correctness/useExhaustiveDependencies: a change of teammate is what closes it.
	useEffect(() => {
		setFocusSchedules(false);
	}, [selectedId]);

	useEffect(() => {
		if (selectedId !== null && roster.length > 0 && !roster.some((one) => one.persona.id === selectedId)) {
			setSelectedId(null);
		}
	}, [roster, selectedId, setSelectedId]);

	// A clicked toast is a teammate asking to be looked at; the shell has
	// already raised the window.
	// One from a desk not on screen opens that desk on that teammate.
	useEffect(
		() =>
			listenToastClicks((payload) => {
				const { deskId, personaId } = toastTarget(payload);
				select(personaId, deskId ?? undefined);
			}),
		[select],
	);

	const toggleInspector = useCallback(
		(schedules = false) => {
			if (selectedId === null) return;
			setPane(null);
			setDock(null);
			setFocusSchedules(schedules);
			setInspector((open) => schedules || !open);
		},
		[selectedId, setPane, setDock, setInspector],
	);
	/* The right-hand pane takes the inspector's place: one of them at a time. */
	const openDock = useCallback((next: DockState) => {
		setInspector(false);
		setDock(next);
	}, [setInspector, setDock]);
	const closeDock = useCallback(() => setDock(null), [setDock]);
	const openThread = useCallback((open: ThreadRef) => {
        if (open.thread.kind === "dm") {
            select(open.thread.key);
            setDock(null);
            if (open.eventId !== undefined) setFocus({ eventId: open.eventId, at: Date.now() });
        } else openDock({ open });
    }, [openDock, select, setDock]);
	const openThreadList = useCallback(() => openDock({ open: null }), [openDock]);
	const openPanel = useCallback(
		(personaId: string, deskId?: string) => {
			if (deskId !== undefined) {
				openOnDesk(deskId, { personaId, inspector: true });
				return;
			}
			select(personaId);
			setDock(null);
			setInspector(true);
		},
		[select, setDock, setInspector],
	);
	const [accountPage, setAccountPage] = useState<string | null>(null);
	useEffect(() => {
		if (pane !== "settings") setAccountPage(null);
	}, [pane]);

	/* A caption or the mark, pressed again with its work already open, closes it. */
	const openWork = useCallback(
		(work: OpenWork) => {
			const was = works[work.personaId];
			if (was !== undefined && sameWork(was, work)) closeWork(work.personaId);
			else setWorks((all) => ({ ...all, [work.personaId]: work }));
		},
		[works, closeWork],
	);

	/* A refusal here is a race — the pane and the menu both stop at three —
	 * so the next roster event is the whole answer. */
	/* A teammate on another desk is pinned and removed on its own desk. */
	const pinTeammate = useCallback(async (personaId: string, slot: number | null, deskId?: string) => {
		const params = slot === null ? { id: personaId } : { id: personaId, slot };
		try {
			if (deskId === undefined) await wire.command("persona.pin", params);
			else await wireFor(deskId).command("persona.pin", params);
		} catch {
			// Nothing to undo: the roster never moved.
		}
	}, []);

	/* The roster's next rows carry the lead, so a refusal leaves nothing to undo. */
	const leadTeammate = useCallback(async (personaId: string, lead: boolean, deskId?: string) => {
		const params = lead ? { id: personaId } : {};
		try {
			if (deskId === undefined) await wire.command("persona.lead", params);
			else await wireFor(deskId).command("persona.lead", params);
		} catch {
			// Clearing the lead of a desk of one is refused; the menu does not offer it.
		}
	}, []);

	const removeTeammate = useCallback(
		async (personaId: string, name: string, deskId?: string) => {
			// The desk is the one the teammate was on when asked about, whatever is on screen by the answer.
			const on = deskId ?? activeDeskId();
			if (!(await confirmRemove(name))) return;
			try {
				if (on !== activeDeskId() && on !== null) {
					await wireFor(on).command("persona.delete", { id: personaId });
					return;
				}
				await wire.command("persona.delete", { id: personaId });
				// Whoever is open by the answer, not whoever was open when asked.
				setSelectedId((open) => (open === personaId ? null : open));
				setSpaces(({ [personaId]: _, ...rest }) => rest);
			} catch {
				// The inspector's own Remove reports a refusal if this fails.
			}
		},
		[setSelectedId, setSpaces],
	);

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

	/* Whether a teammate has ever been made in this room, as `welcome`'s
	 * setUp reads it off the room's stream. A room with a teammate is set up
	 * by definition, so the core is asked only while the roster is empty —
	 * on the first snapshot, after the last teammate is deleted, after a
	 * reconnect — and the answer before stands while the next is on its way.
	 * Null until there is one, so neither screen flashes in ahead of it. */
	const [setUp, setSetUp] = useState<boolean | null>(null);
	const empty = rosterLoaded && roster.length === 0;
	useEffect(() => {
		if (!rosterLoaded) return;
		if (!empty) {
			setSetUp(true);
			return;
		}
		if (connection !== "open") return;
		let current = true;
		wire
			.command("welcome", {})
			.then((state) => current && setSetUp(state.setUp))
			// The welcome reads the same command and says what went wrong.
			.catch(() => current && setSetUp(false));
		return () => {
			current = false;
		};
	}, [rosterLoaded, empty, connection]);
	/* An empty room nobody has set up is the welcome alone, in a bare
	 * window; until the core has said which room this is, it is that bare
	 * window with nothing in it yet. */
	/* Another desk's teammates in the window are a room to be in, with its
	 * rail, whether or not this desk has been set up. */
	const welcome = empty && setUp !== true && Object.values(elsewhere).every((entries) => entries.length === 0);
	const call = useCall();
	const callState = useCallSnapshot(call);
	const callPhase = callState.phase;
	const calling = call !== null && callPhase !== "ended";
	const { available: voice, directCalls } = useVoiceSupport(connection);
	const nameOf = useCallback((personaId: string) => roster.find((one) => one.persona.id === personaId)?.persona.name, [roster]);
	const callTeammate = useCallback(
		(entry: RosterEntry) =>
			void startCall(nameOf, { personaId: entry.persona.id, name: entry.persona.name, avatarHash: entry.persona.avatar?.hash }),
		[nameOf],
	);

	useWindowHotkeys({
		roster,
		elsewhere,
		selectedId,
		selected,
		pane,
		setPane,
		dock,
		setDock,
		closeDock,
		inspector,
		setInspector,
		workOf,
		closeWork,
		select,
		closePane,
		togglePane,
		newTeammate,
		toggleInspector,
		toggleRail,
		setSearchAsk,
		setSettingsSection,
		calling,
		voice,
		directCalls,
		nameOf,
		callTeammate,
	});
	/* A narrow window has room for faces beside the pane and no more. */
	const faces = narrow || railSize.compact;
	/* Settings' sections have no faces to fall back to: they stand at the
	 * names' width, and at the narrowest of it in a narrow window. */
	const settingsWidth = narrow ? RAIL_MIN : railSize.width;
	/* The right-hand pane lies over the conversation once it would leave it
	 * too narrow to read beside it. */
	const dockOverlay = dock !== null && pane !== "settings" && paneOverlays;


	/* The work card shows only on its own teammate's conversation: not over
	 * Settings or another pane, and not on a teammate who has none open. */
	const workCard = (entry: RosterEntry, open: OpenWork) =>
		open.processId !== undefined ? (
			<ProcessWork
				key={`process-${entry.persona.id}-${open.processId}`}
				personaId={entry.persona.id}
				processId={open.processId}
				process={entry.session.processes?.find((process) => process.id === open.processId)}
				docked={dockWork}
				onClose={() => closeWork(entry.persona.id)}
			/>
		) : (
			<Work
				key={`work-${entry.persona.id}`}
				open={open}
				name={entry.persona.name}
				live={shownState(entry.session) === "thinking"}
				docked={dockWork}
				onClose={() => closeWork(entry.persona.id)}
			/>
		);
	const floatWork = pane === null && !welcome && selected !== null && workOf !== undefined && !dockWork && !dockOverlay ? workCard(selected, workOf) : null;


	// A call to a teammate is the conversation's inset card, or a strip atop
	// a pane with no conversation. A call to the desk grows out of the rail's
	// dock, whose key places it; with the rail closed or showing Settings it
	// is the strip too, and the key is the title bar's.
	const teamSearch = (placement: "dock" | "beside" | "under") => (
		<TeamSearch
			roster={roster}
			selectedId={pane === null ? selectedId : null}
			placement={placement}
			ask={searchAsk}
			onPick={(personaId, eventId) => {
				select(personaId);
				if (eventId !== undefined) setFocus({ eventId, at: Date.now() });
			}}
		/>
	);
	const conversationShown = pane === null && selected !== null;
	const teamRail = railSize.open && pane !== "settings";
	const deskCalling = call !== null && call.target === undefined;
	// A failure needs its sentence and choice: use the pane when the rail is faces only.
	const deskInRail = deskCalling && teamRail && !(faces && callState.legacyAvailable);
	const callBar = call === null || deskInRail ? undefined : (
		<CallBar
			call={call}
			names={nameOf}
			roster={roster}
			onOpenTeammate={(personaId) => select(personaId, call.deskId ?? undefined)}
			bare={conversationShown && call.target?.personaId === selected?.persona.id}
			alone={!conversationShown}
		/>
	);
	const deskCallKey = (className?: string) => (
		<DeskCallKey call={call} available={voice} onCall={() => void startCall(nameOf)} {...(className !== undefined ? { className } : {})} />
	);
	return (
		<div className="flex h-full flex-col">
			<Titlebar
				bare={welcome && pane === null}
				rail={{ open: railSize.open, onToggle: toggleRail }}
				search={teamSearch("under")}
				deskKey={deskCallKey("control btn-icon titlebar-desk-call")}
				onNew={newTeammate}
				onSettings={() => togglePane("settings")}
				settingsOpen={pane === "settings"}
			/>
			{platform() === "linux" && <WindowEdges />}
			<ServerFiles />
			{welcome && pane === null ? (
				// A room never set up is the welcome alone: no rail, nothing to pick in it yet.
				<div className="flex min-h-0 flex-1 p-2 pt-0">
					{setUp === false && (
						<Welcome models={models} onCreated={select} onConnectServer={desk?.kind === "local" ? () => togglePane("add-desk") : null} />
					)}
				</div>
			) : (
			<div className="flex min-h-0 flex-1 gap-2 p-2 pt-0">
			{!railSize.open ? null : pane === "settings" ? (
				<Suspense fallback={null}>
					<SettingsRail
						section={settingsSection}
						onSection={(section) => {
							setAccountPage(null);
							setSettingsSection(section);
						}}
						onBack={closePane}
						onDesk={(deskId) => openOnDesk(deskId, { pane: "settings", section: settingsSection })}
						width={settingsWidth}
					/>
				</Suspense>
			) : (
			<Rail
				entries={roster}
				loaded={rosterLoaded}
				selectedId={selectedId}
				seen={seen}
				connection={connection}
				onSelect={select}
				elsewhere={elsewhere}
				onEdit={openPanel}
				onDelete={(id, name, deskId) => void removeTeammate(id, name, deskId)}
				onPin={(id, slot, deskId) => void pinTeammate(id, slot, deskId)}
				onLead={(id, lead, deskId) => void leadTeammate(id, lead, deskId)}
				onAddDesk={() => togglePane("add-desk")}
				width={faces ? RAIL_FACES : railSize.width}
				compact={faces}
				search={teamSearch(faces ? "beside" : "dock")}
				deskKey={deskCallKey()}
				deskCard={deskInRail && call !== null ? <DeskCallCard call={call} roster={roster} compact={faces} /> : undefined}
			/>
			)}
			{!narrow && <RailEdge size={railSize} onSize={setRailSize} />}

			<main ref={mainRef} className="@container flex min-w-0 flex-1 flex-col gap-0">
				<DeskBand onAddDesk={() => togglePane("add-desk")} />
				{rosterLoaded && (
					<LeadPrompt deskId={desk?.id ?? ""} roster={roster} deskCalling={deskCalling} onLead={(id) => void leadTeammate(id, true)} />
				)}
				{/* A pane with no conversation has no band of its own to hang the call from. */}
				{!conversationShown && callBar}
				<div className="relative flex min-h-0 min-w-0 flex-1 gap-2">
				{pane === "settings" ? (
					<Suspense fallback={null}>
						<Settings
							section={settingsSection}
							roster={roster}
							account={accountPage ?? undefined}
							onSection={setSettingsSection}
							onAddDesk={() => togglePane("add-desk")}
						/>
					</Suspense>
				) : pane === "add-desk" ? (
					<AddDesk onClose={closePane} />
				) : pane === "new-teammate" ? (
					<NewTeammate
							models={models}
							onCreated={select}
							onClose={closePane}
							onDesk={(deskId) => openOnDesk(deskId, { pane: "new-teammate" })}
							onProviders={() => {
								setSettingsSection("providers");
								setPane("settings");
							}}
						/>
				) : selected ? (
					<>
						<Conversation
							key={selected.persona.id}
							entry={selected}
							models={models}
							onSaid={setSaid}
							onCall={directCalls ? () => callTeammate(selected) : undefined}
							onHangUp={calling && call?.target?.personaId === selected.persona.id ? closeCall : undefined}
							callBar={callBar}
							onProviders={() => {
								setSettingsSection("providers");
								setPane("settings");
							}}
							onChooseAccount={() => openPanel(selected.persona.id)}
							roster={roster}
							jobs={jobs.filter((job) => job.personaId === selected.persona.id)}
							said={said}
							inspectorOpen={inspector}
							focus={focus}
							onToggleInspector={() => toggleInspector()}
							onOpenSchedules={() => toggleInspector(true)}
							onDelete={() => void removeTeammate(selected.persona.id, selected.persona.name)}
							onOpenThread={openThread}
							onOpenThreadList={openThreadList}
							onOpenWork={(blockId) => openWork({ personaId: selected.persona.id, blockId })}
							workOpen={workOf?.processId === undefined ? workOf?.blockId : undefined}
							onOpenProcess={(processId) => openWork({ personaId: selected.persona.id, blockId: null, processId })}
							processOpen={workOf?.processId}
							threadOpen={dock?.open?.thread}
							paneOpen={dock !== null}
							{...(dockWork && workOf !== undefined ? { dock: workCard(selected, workOf) } : {})}
						/>
						{inspector && (
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
								onAccountSetup={(accountId) => {
									setAccountPage(accountId);
									setSettingsSection("providers");
									setPane("settings");
								}}
							/>
						)}
					</>
				) : empty ? (
					<EmptyRoom onNew={newTeammate} />
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
					{dock !== null && pane !== "settings" && (
						<Dock
							state={dock}
							onState={(next) => next.open?.thread.kind === "dm" ? openThread(next.open) : setDock(next)}
							onClose={closeDock}
							entry={selected}
							roster={roster}
							width={dockWidth}
							onWidth={setDockWidth}
							overlay={dockOverlay}
							onOpenTeammate={select}
						/>
					)}
					{/* What floats over the window runs down its right edge: a turn's work at the top, a new version at the bottom. */}
				{(floatWork !== null || pane !== "settings") && (
					<div
						className="float-stack"
						style={
							dock !== null && pane !== "settings" && !dockOverlay
								? { right: dockWidth + 24 }
								: inspector && selected !== null && pane === null
									? { right: INSPECTOR_WIDTH + 24 }
									: undefined
						}
					>
						{floatWork}
						{/* Settings › Updates says it already, with the notes and the install. */}
						{pane !== "settings" && <UpdateFloat onOpen={() => {
							setSettingsSection("updates");
							setPane("settings");
						}} />}
					</div>
				)}
				</div>
			</main>
			</div>
			)}
		</div>
	);
}

/** The teammate's pane (`.inspector` in index.css); what floats keeps clear of it. */
const INSPECTOR_WIDTH = 320;

/** The width of the window's main column below which a work card docks. */
const WORK_DOCK_BELOW = 960;

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
