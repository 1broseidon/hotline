import { useCallback, useEffect, useRef, useState, type SetStateAction } from "react";
import type { DockState } from "./components/Dock";
import type { SettingsSection } from "./components/Settings";
import { chatWindow } from "./chatWindow";
import { activeDeskId, allDesks, deskFilter, deskKey, LOCAL_DESK, setActiveDesk } from "./desks";

/** What stands in the conversation's place: a room-wide pane, or nothing. */
export type Pane = "settings" | "new-teammate" | "add-desk" | null;

/**
 * What the window opens on when it next shows a desk: the pane, and whether
 * the teammate's own pane is open beside it. Set by `openOnDesk` just
 * before the window remounts on another desk, and spent by that mount.
 */
let arriving: { pane: Pane; inspector: boolean; section?: SettingsSection } = { pane: null, inspector: false };

/**
 * Opens something on another desk: a teammate picked in the rail, its pane,
 * or the new-teammate pane there. The rail lists every desk's teammates
 * (BRO-178), and the screen follows the teammate: its desk becomes the one
 * the window talks to, and nothing needs switching by hand.
 */
export function openOnDesk(deskId: string, open: { personaId?: string; pane?: Pane; inspector?: boolean; section?: SettingsSection }) {
	if (open.personaId !== undefined && chatWindow !== null) chatSelected = open.personaId;
	else if (open.personaId !== undefined) {
		try {
			localStorage.setItem(deskKey(SELECTED_KEY, deskId), open.personaId);
		} catch {
			// Private mode: the desk opens on whoever it had open.
		}
	}
	arriving = { pane: open.pane ?? null, inspector: open.inspector ?? false, ...(open.section === undefined ? {} : { section: open.section }) };
	setActiveDesk(deskId);
}

/**
 * Where the window stands: the teammate open, the pane in the
 * conversation's place, Settings' section, and what each teammate's screen
 * has open beside its conversation.
 */
export function useNavigation() {
	const [selectedId, setSelectedId] = useState<string | null>(loadSelected);
	/* What each teammate's screen has open beside the conversation: its
	 * threads pane and the thread in it, or its own pane. Like the work card
	 * below it belongs to them, so leaving a teammate and coming back finds
	 * their screen as it was, and another teammate's as theirs was. */
	const [spaces, setSpaces] = useState<Record<string, Space>>(() => {
		const id = loadSelected();
		return arriving.inspector && id !== null ? { [id]: { ...NO_SPACE, inspector: true } } : {};
	});
	const selectedRef = useRef(selectedId);
	selectedRef.current = selectedId;
	const shapeSpace = useCallback((change: (was: Space) => Partial<Space>) => {
		const personaId = selectedRef.current;
		if (personaId === null) return;
		setSpaces((all) => {
			const was = all[personaId] ?? NO_SPACE;
			return { ...all, [personaId]: { ...was, ...change(was) } };
		});
	}, []);
	const space = (selectedId !== null ? spaces[selectedId] : undefined) ?? NO_SPACE;
	const dock = space.dock;
	const inspector = space.inspector;
	const setDock = useCallback(
		(next: SetStateAction<DockState | null>) => shapeSpace((was) => ({ dock: typeof next === "function" ? next(was.dock) : next })),
		[shapeSpace],
	);
	const setInspector = useCallback(
		(next: SetStateAction<boolean>) => shapeSpace((was) => ({ inspector: typeof next === "function" ? next(was.inspector) : next })),
		[shapeSpace],
	);
	const [pane, setPane] = useState<Pane>(() => arriving.pane);
	useEffect(() => {
		arriving = { pane: null, inspector: false };
	}, []);
	const [settingsSection, setSettingsSection] = useState<SettingsSection>(() => arriving.section ?? "general");

	useEffect(() => {
		saveSelected(selectedId);
	}, [selectedId]);

	/* A teammate on another desk opens there: the window follows it. */
	const select = useCallback((personaId: string, deskId?: string) => {
		if (deskId !== undefined && deskId !== activeDeskId()) {
			openOnDesk(deskId, { personaId });
			return;
		}
		selectedRef.current = personaId;
		setSelectedId(personaId);
		setPane(null);
	}, []);

	const closePane = useCallback(() => {
		setPane(null);
	}, []);
	const togglePane = useCallback(
		(id: Exclude<Pane, null>) => {
			if (pane === id) {
				closePane();
				return;
			}
			setPane(id);
		},
		[pane, closePane],
	);
	/* A new teammate is asked which desk it lives on when the window holds
	 * more than one: the pane opens on the desk the rail is narrowed to, or
	 * else on this computer's, and its own picker moves it to another. */
	const newTeammate = useCallback(() => {
		const wanted = deskFilter() ?? (allDesks().some((one) => one.id === LOCAL_DESK) ? LOCAL_DESK : null);
		if (pane !== "new-teammate" && wanted !== null && wanted !== activeDeskId()) {
			openOnDesk(wanted, { pane: "new-teammate" });
			return;
		}
		togglePane("new-teammate");
	}, [pane, togglePane]);

	return {
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
	};
}

/** The open teammate survives a reload, which is what makes the tape
 * subscribe able to race wire.connect() — see watchWhenOpen in tape.ts. */
const SELECTED_KEY = "hotline.rail.selected";

/* A chat in a window of its own opens on its teammate, and what it opens after is not saved. */
let chatSelected: string | null = chatWindow?.personaId ?? null;

function loadSelected(): string | null {
	if (chatWindow !== null) return chatSelected;
	try {
		const id = localStorage.getItem(deskKey(SELECTED_KEY));
		return id !== null && id !== "" ? id : null;
	} catch {
		return null;
	}
}

function saveSelected(id: string | null): void {
	if (chatWindow !== null) {
		chatSelected = id;
		return;
	}
	try {
		if (id === null) localStorage.removeItem(deskKey(SELECTED_KEY));
		else localStorage.setItem(deskKey(SELECTED_KEY), id);
	} catch {
		// Quota, private mode.
	}
}

/** What a teammate's screen has open beside its conversation. */
type Space = { dock: DockState | null; inspector: boolean };
const NO_SPACE: Space = { dock: null, inspector: false };
