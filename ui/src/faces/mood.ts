import { useEffect, useState, useSyncExternalStore } from "react";
import type { Look, Mood, RosterEntry } from "../generated/contract";
import { useFacePrefs } from "./prefs";

/**
 * What an animated face shows, worked out from what the window already knows
 * about the teammate: its session state, whether it waits on you, and whether
 * you are typing to it. Nothing here reaches the desk; a mood is a reading of
 * the roster, not part of it.
 */
export type { Mood };

/** How long a finished turn shows its bounce before the face settles. */
const DONE_FOR = 2600;
/** How long after the last keystroke the face stops watching you type. */
const TYPING_FOR = 2500;
/** How long a teammate at rest stays awake. */
const AWAKE_FOR = 5 * 60_000;

type Seen = { state: RosterEntry["session"]["state"]; waiting: boolean; working: boolean };

const seen = new Map<string, Seen>();
/** Each teammate's look and picture, so a face anywhere on screen draws from the record without every caller passing it. */
const looks = new Map<string, { look: Look | undefined; hash: string | undefined }>();
const typing = new Map<string, { at: number; caret: number }>();
const listeners = new Set<() => void>();
let version = 0;

function changed() {
	version++;
	for (const listener of listeners) listener();
}

/** The window's roster, so every face on screen reads the same state. */
export function noteRoster(roster: readonly RosterEntry[]) {
	for (const entry of roster) {
		seen.set(entry.persona.id, { state: entry.session.state, waiting: entry.waiting, working: entry.activity !== undefined });
		looks.set(entry.persona.id, { look: entry.persona.look, hash: entry.persona.avatar?.hash });
	}
	changed();
}

/** A keystroke in the composer; `caret` is how far along the line it is, 0 to 1, for the eyes to follow. */
export function noteTyping(personaId: string, caret: number) {
	typing.set(personaId, { at: Date.now(), caret: Math.min(1, Math.max(0, caret)) });
	changed();
}

function subscribe(listener: () => void) {
	watchFocus();
	listeners.add(listener);
	return () => listeners.delete(listener);
}

/** A teammate's look as the roster last had it. */
export function useLook(personaId: string): Look | undefined {
	useSyncExternalStore(subscribe, () => version, () => version);
	return looks.get(personaId)?.look;
}

/** Where the eyes look while you type, or null when you are not. */
export function typingCaret(personaId: string): number | null {
	const typed = typing.get(personaId);
	return typed !== undefined && Date.now() - typed.at < TYPING_FOR ? typed.caret : null;
}

/** Whether the window is the one in front, so a face can doze while you are elsewhere. */
let focused = true;
let focusedAt = 0;
let watchingFocus = false;

/** Listens for the window's focus from the first face on, so importing this module touches nothing. */
function watchFocus() {
	if (watchingFocus || typeof window.addEventListener !== "function") return;
	watchingFocus = true;
	focused = typeof document.hasFocus !== "function" || document.hasFocus();
	window.addEventListener("focus", () => {
		focused = true;
		focusedAt = Date.now();
		changed();
	});
	window.addEventListener("blur", () => {
		focused = false;
		changed();
	});
	// A webview does not always say focus when its app comes back to the front;
	// a touch or a key in the window says it as surely.
	for (const kind of ["pointerdown", "pointermove", "keydown"] as const) window.addEventListener(kind, back, { capture: true, passive: true });
}

function back() {
	if (focused) return;
	focused = true;
	focusedAt = Date.now();
	changed();
}

/**
 * Whether the window is in front. A face in a window in the background
 * stands still, since no one is watching it move and moving costs power.
 */
export function useWindowFocused(): boolean {
	useEffect(watchFocus, []);
	useSyncExternalStore(subscribe, () => version, () => version);
	return focused;
}

/** Back in front: he stays asleep this long, then wakes and looks at you this long. */
const WAKING_AFTER = 1000;
const WOKEN_FOR = 1800;

/**
 * A teammate at rest in a window you are looking at plays one of these now
 * and then, so the face has a temper and not just a blink. Every face of the
 * same teammate picks the same one at the same moment: the pick is a hash of
 * the teammate and the time slot, not a random draw per face.
 */
const FLOURISHES: readonly Mood[] = ["grumpy", "crossed-arms", "needy", "grumpy", "think"];
const FLOURISH_EVERY = 12_000;
const FLOURISH_FOR = 4_000;

function flourishAt(personaId: string, now: number): Mood | null {
	const slot = Math.floor(now / FLOURISH_EVERY);
	if (now - slot * FLOURISH_EVERY > FLOURISH_FOR) return null;
	let hash = slot;
	for (let index = 0; index < personaId.length; index++) hash = (hash * 31 + personaId.charCodeAt(index)) % 1_000_003;
	// One slot in three stays plain, so the flourishes stay a surprise.
	if (hash % 3 === 0) return null;
	return FLOURISHES[hash % FLOURISHES.length] ?? null;
}

/** The mood a teammate's face should show now. Re-renders when the roster, the composer or the window's focus changes, and once a second for the timers. */
export function useMood(personaId: string): Mood {
	useSyncExternalStore(subscribe, () => version, () => version);
	const { doze } = useFacePrefs();
	const state = seen.get(personaId);
	const working = state?.state === "thinking";
	const [doneUntil, setDoneUntil] = useState(0);
	const [restingSince, setRestingSince] = useState(() => Date.now());
	const [now, setNow] = useState(() => Date.now());

	// A turn that ends is the bounce; anything that happens wakes the face.
	const [wasWorking, setWasWorking] = useState(working);
	if (wasWorking !== working) {
		setWasWorking(working);
		if (!working) setDoneUntil(Date.now() + DONE_FOR);
		setRestingSince(Date.now());
	}
	const caret = typingCaret(personaId);
	if (caret !== null && now - restingSince > 1000) setRestingSince(now);

	// The timers: a bounce ending, typing ending, waking, a flourish, falling asleep.
	useEffect(() => {
		const timer = setInterval(() => {
			if (!focused && document.hasFocus()) back();
			setNow(Date.now());
		}, 500);
		return () => clearInterval(timer);
	}, []);

	if (state === undefined) return "idle";
	if (state.waiting) return "needy";
	if (state.state === "error") return "grumpy";
	if (state.state === "stopped") return "crossed-arms";
	if (working) return state.working ? "work" : "think";
	if (state.state === "starting") return "think";
	if (now < doneUntil) return "done";
	// Typing to him is him typing along: the working paws, eyes on your caret.
	if (caret !== null) return "work";
	if (doze) {
		if (!focused) return "sleep";
		if (now - focusedAt < WAKING_AFTER) return "sleep";
		if (now - focusedAt < WAKING_AFTER + WOKEN_FOR) return "attentive";
	}
	if (state.state === "idle" || now - restingSince > AWAKE_FOR) return "sleep";
	return flourishAt(personaId, now) ?? "idle";
}

/** The least time a mood stays on the face once it starts, so a quick turn still plays its whole animation. */
const HOLD: Partial<Record<Mood, number>> = { done: DONE_FOR, think: 1800, work: 1800, needy: 2500, "crossed-arms": 2500, grumpy: 2000 };

/** `wanted`, but never cutting the mood on screen short of its HOLD. */
export function useHeldMood(wanted: Mood): Mood {
	const [shown, setShown] = useState<{ mood: Mood; since: number }>(() => ({ mood: wanted, since: Date.now() }));
	const [, wake] = useState(0);
	const left = shown.since + (HOLD[shown.mood] ?? 0) - Date.now();
	if (wanted !== shown.mood && left <= 0) setShown({ mood: wanted, since: Date.now() });
	useEffect(() => {
		if (wanted === shown.mood || left <= 0) return;
		const timer = setTimeout(() => wake((tick) => tick + 1), left);
		return () => clearTimeout(timer);
	}, [wanted, shown, left]);
	return shown.mood;
}
