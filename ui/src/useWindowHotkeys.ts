import { useCallback, useEffect, useRef, type Dispatch, type SetStateAction } from "react";
import { matchChord } from "./chords";
import type { OpenWork } from "./components/Work";
import { activeDeskId, allDesks, deskFilter } from "./desks";
import { hotkeyRegistrar, onHotkey, useHotkeys, useRecording, type HotkeyId, type KeyState } from "./hotkeys";
import { listenMenu, openLink, showWindow } from "./native";
import { shownTeam, teamOrder } from "./pins";
import type { useNavigation } from "./useNavigation";
import { closeCall, startCall } from "./voice/call";
import { requestDictation, useDictationAvailable } from "./voice/dictation";
import type { RosterEntry } from "./wire";

type Navigation = ReturnType<typeof useNavigation>;

/**
 * Everything the window does from a key or a menu: its chords, the menu
 * bar's items, and the shortcuts heard anywhere on this computer.
 */
export function useWindowHotkeys({
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
}: Pick<
	Navigation,
	"selectedId" | "pane" | "setPane" | "dock" | "setDock" | "inspector" | "setInspector" | "select" | "closePane" | "togglePane" | "newTeammate" | "setSettingsSection"
> & {
	roster: RosterEntry[];
	elsewhere: Record<string, RosterEntry[]>;
	selected: RosterEntry | null;
	closeDock(): void;
	workOf: OpenWork | undefined;
	closeWork(personaId: string): void;
	toggleInspector(schedules?: boolean): void;
	toggleRail(): void;
	setSearchAsk: Dispatch<SetStateAction<number>>;
	calling: boolean;
	voice: boolean;
	directCalls: boolean;
	nameOf(personaId: string): string | undefined;
	callTeammate(entry: RosterEntry): void;
}) {
	/* ⌃1–9 count along the rail as it is drawn: every desk it lists. */
	const seatOf = useCallback(
		(seat: number) => teamOrder(shownTeam(allDesks(), activeDeskId() ?? "", deskFilter(), roster, elsewhere)).all[seat - 1],
		[roster, elsewhere],
	);

	// Chords live in chords.ts so Help cannot list a key the window does
	// not hear. Escape closes whatever is on top: the search, then a pane,
	// then the inspector.
	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			const chord = matchChord(event);
			if (chord === "close") {
				if (pane !== null) {
					event.preventDefault();
					closePane();
					return;
				}
				if (dock !== null && !(event.target as HTMLElement | null)?.closest("textarea, input")) {
					// One step back, then out: a thread to the list, the list to the window.
					event.preventDefault();
					if (dock.open !== null) setDock({ open: null });
					else closeDock();
					return;
				}
				if (selectedId !== null && workOf !== undefined && !(event.target as HTMLElement | null)?.closest("textarea, input")) {
					event.preventDefault();
					closeWork(selectedId);
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
				if (takeChord()) newTeammate();
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
				event.preventDefault();
				setSearchAsk((count) => count + 1);
				return;
			}
			if (chord === "sidebar") {
				event.preventDefault();
				if (takeChord()) toggleRail();
				return;
			}
			if (chord === null || !chord.startsWith("teammate-")) return;
			const seat = Number(chord.slice("teammate-".length));
			const row = seatOf(seat);
			if (!row) return;
			event.preventDefault();
			select(row.entry.persona.id, row.deskId);
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [selectedId, pane, dock, inspector, workOf, closeWork, select, seatOf, closePane, closeDock, togglePane, newTeammate, toggleInspector, toggleRail]);

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
				if (takeChord()) newTeammate();
				return;
			}
			if (id === "teammate") {
				if (takeChord()) toggleInspector();
				return;
			}
			if (id === "search") {
				setSearchAsk((count) => count + 1);
				return;
			}
			// About and the shortcuts are sections of Settings.
			if (id === "about" || id === "shortcuts") {
				setSettingsSection(id);
				setPane("settings");
				return;
			}
			if (id === "github") {
				void openLink("https://github.com/1broseidon/hotline");
				return;
			}
			if (id.startsWith("teammate-")) {
				const row = seatOf(Number(id.slice("teammate-".length)));
				if (row) select(row.entry.persona.id, row.deskId);
			}
		});
	}, [selectedId, pane, select, seatOf, togglePane, newTeammate, toggleInspector, toggleRail]);

	/* The shortcuts heard anywhere on this computer (hotkeys.ts). Dictate
	 * brings the window forward and hands its press and release to the open
	 * conversation's composer, or the last teammate's when a pane stands in
	 * its place, which tells a tap from a hold. Call your agent calls the
	 * open teammate and Call the desk calls the desk, each coming forward to
	 * do it; either hangs up a live call where the person is. */
	const hotkeyPressed = useRef<(id: HotkeyId, state: KeyState) => void>(() => {});
	hotkeyPressed.current = (id, state) => {
		if (id === "dictate") {
			if (selected === null) return;
			if (state === "Pressed") {
				void showWindow();
				setPane(null);
			}
			requestDictation(state === "Pressed" ? "down" : "up");
			return;
		}
		if (state !== "Pressed") return;
		if (calling) {
			closeCall();
			return;
		}
		if (id === "desk") {
			if (!voice) return;
			void showWindow();
			void startCall(nameOf);
			return;
		}
		if (selected === null || !directCalls) return;
		void showWindow();
		callTeammate(selected);
	};
	useEffect(() => onHotkey((id, state) => hotkeyPressed.current(id, state)), []);
	const bindings = useHotkeys();
	const recordingKeys = useRecording();
	const dictationHere = useDictationAvailable();
	useEffect(() => {
		void hotkeyRegistrar()?.sync(recordingKeys ? {} : { dictate: dictationHere ? bindings.dictate : "", conversation: bindings.conversation, desk: bindings.desk });
	}, [bindings, recordingKeys, dictationHere]);
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
