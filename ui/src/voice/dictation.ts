import { useEffect, useState, useSyncExternalStore } from "react";
import { deskEngine, deskHears, useDeskHears } from "./desk";
import { hearOnThisMac } from "./hearing";
import { type DeviceTranscription, deviceTranscription, hearsOnThisMac, rawEquivalentDb } from "./transcription";

/**
 * Dictation into the composer: an engine hears the person and the words go
 * into the field, never to a teammate until the person sends them. The
 * engine works one utterance per session; a pause it takes for the end of
 * an utterance starts a fresh session and the words carry on after the last
 * ones, until the person stops. Kept out of React so the merging and the
 * engine's order of events can be tested without a window.
 *
 * There are two engines: this Mac's (`macEngine`, over transcription.ts),
 * and the desk's own model, heard over the wire (`deskEngine`, desk.ts).
 * A Mac uses its own unless the person picked something else for hearing
 * and the desk has a model; every other window uses the desk's. The
 * controller, the composer and the meter do not change with the engine.
 */

/** What dictation needs from a speech engine. */
export interface DictationEngine {
	/** Never asks permission or opens the microphone. Never rejects. */
	capability(): Promise<{ available: boolean }>;
	/** Asks for whatever the engine needs to hear. Never rejects. */
	permit(): Promise<boolean>;
	/** A fresh utterance. False means it did not start. */
	start(onEvent: (event: DictationEvent) => void): Promise<boolean>;
	/** Closes the microphone and answers the utterance's complete final text. */
	stop(): Promise<string>;
	/** At once stops the session and every callback of it. */
	cancel(): Promise<void>;
}

/** Text events are the whole utterance so far, never a delta. A level is already 0 to 1 (`levelFromDbfs`). */
export type DictationEvent =
	| { type: "partial" | "final"; text: string }
	| { type: "level"; level: number }
	| { type: "error"; message: string }
	| { type: "ended"; reason: "final" | "no-speech" | "cancelled" | "error" };

/** This Mac's engine as a dictation engine: the same, with its dBFS made a level. */
export function macEngine(speech: DeviceTranscription): DictationEngine {
	return {
		capability: () => speech.capability(),
		permit: () => speech.permit(),
		start: (onEvent) =>
			speech.start((event) => onEvent(event.type === "level" ? { type: "level", level: levelFromDbfs(rawEquivalentDb(event.levelDb)) } : event)),
		stop: () => speech.stop(),
		cancel: () => speech.cancel(),
	};
}

type DictationPhase = "idle" | "starting" | "listening" | "finishing";

type DictationView = {
	phase: DictationPhase;
	/** One sentence for a person when dictation stopped on its own or would not start. */
	error: string | null;
};

/** The field dictation writes into. */
type DictationField = {
	read(): string;
	write(text: string): void;
	/** A dictation stopped by the person put its words in the field: these, without what was there before. */
	done?(heard: string): void;
};

export const DICTATION_UNAVAILABLE = "Dictation isn't available yet. Download a speech model for the desk in Settings › Providers.";
export const DICTATION_DENIED =
	"Hotline can't hear you. Allow Hotline under Speech Recognition and Microphone in System Settings › Privacy & Security, then try again.";
const DICTATION_NOT_STARTED = "Dictation couldn't start. Try again.";
const DICTATION_STOPPED = "Dictation stopped. What it heard is in the field.";

/** The microphone's loudness in dBFS from a quiet room to a raised voice, which a level spreads across 0 to 1. */
const QUIET_DB = -48;
const LOUD_DB = -10;

/**
 * Loudness as the eye reads it: even steps of decibels, not of pressure,
 * so speech moves a meter and a quiet room leaves it still.
 */
export function levelFromDbfs(db: number): number {
	if (!Number.isFinite(db)) return 0;
	return Math.min(1, Math.max(0, (db - QUIET_DB) / (LOUD_DB - QUIET_DB)));
}

/** A meter rises with a word in a few frames and falls back over a breath. */
const ATTACK_MS = 40;
const RELEASE_MS = 260;

/**
 * One step of a meter's level towards the latest reading, `elapsedMs`
 * after the last step: quick up, slow down, the same at any frame rate.
 */
export function followLevel(shown: number, target: number, elapsedMs: number): number {
	const time = target > shown ? ATTACK_MS : RELEASE_MS;
	return shown + (target - shown) * (1 - Math.exp(-Math.max(0, elapsedMs) / time));
}

const NOT_DICTATING: DictationView = { phase: "idle", error: null };

/** Engines whose permission was granted while the app runs; `permit` may download a model, so it is asked once. */
const permitted = new WeakSet<DictationEngine>();

/** The engine has one microphone: a second dictation lets the first go. */
let current: Dictation | null = null;

/** The text the field shows: what was there before, then the words heard, with one space between. */
export function joinDictated(before: string, heard: readonly string[]): string {
	const words = heard.map((part) => part.trim()).filter((part) => part !== "").join(" ");
	if (words === "") return before;
	if (before === "" || /\s$/.test(before)) return before + words;
	return `${before} ${words}`;
}

export class Dictation {
	private state: DictationView = NOT_DICTATING;
	private readonly listeners = new Set<() => void>();
	private readonly levels = new Set<(level: number) => void>();
	/** Bumped by every start, stop and cancel, so a late answer from the engine is dropped. */
	private run = 0;
	/** Bumped by every engine session, so a session the engine ended stops talking. */
	private session = 0;
	/** The field as it was when listening began, or null before then. */
	private before: string | null = null;
	/** Words of the sessions the engine already ended. */
	private committed: string[] = [];
	/** The open session's words: each event is the whole of them. */
	private part = "";
	/** A session the engine is still opening, which a stop waits for so its microphone is not left open. */
	private opening: Promise<unknown> = Promise.resolve();

	constructor(
		private readonly engine: DictationEngine,
		private readonly field: DictationField,
	) {}

	get view(): DictationView {
		return this.state;
	}

	readonly watch = (listener: () => void): (() => void) => {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	};

	/** The microphone's level, 0 to 1, as the engine reports it: unsmoothed, and outside React's renders. */
	readonly watchLevel = (listener: (level: number) => void): (() => void) => {
		this.levels.add(listener);
		return () => this.levels.delete(listener);
	};

	/** The microphone key and the shortcut: start, stop and keep the words, or give up a start still waiting. */
	toggle(): void {
		if (this.state.phase === "idle") void this.start();
		else if (this.state.phase === "starting") this.cancel();
		else if (this.state.phase === "listening") void this.stop();
	}

	async start(): Promise<void> {
		if (this.state.phase !== "idle") return;
		if (current !== null && current !== this) current.cancel();
		current = this;
		const run = ++this.run;
		this.before = null;
		this.set({ phase: "starting", error: null });
		if (!permitted.has(this.engine)) {
			const capability = await this.engine.capability();
			if (run !== this.run) return;
			if (!capability.available) return this.fail(DICTATION_UNAVAILABLE);
			const granted = await this.engine.permit();
			if (run !== this.run) return;
			if (!granted) return this.fail(DICTATION_DENIED);
			permitted.add(this.engine);
		}
		this.before = this.field.read();
		this.committed = [];
		await this.listen(run);
	}

	/** Closes the microphone and puts the complete words in the field. Never sends them. */
	async stop(): Promise<void> {
		if (this.state.phase === "starting") return this.cancel();
		if (this.state.phase !== "listening") return;
		const run = this.run;
		this.session++;
		this.set({ phase: "finishing" });
		this.level(0);
		let final: string;
		try {
			await this.opening;
			if (run !== this.run) return;
			final = await this.engine.stop();
		} catch (error) {
			if (run !== this.run) return;
			// The words on screen stay: the person saw them and can mend them.
			this.fail(error instanceof Error ? error.message : String(error));
			return;
		}
		if (run !== this.run) return;
		this.part = final;
		this.show();
		const heard = joinDictated("", [...this.committed, final]);
		this.end(null);
		this.field.done?.(heard);
	}

	/** Lets the session go and puts the field back as it was. */
	cancel(): void {
		if (this.state.phase === "idle") return;
		const before = this.before;
		this.end(null);
		void this.engine.cancel().catch(() => {});
		if (before !== null) this.field.write(before);
	}

	/** Lets the session go and leaves the field alone: the field is someone else's now. */
	release(): void {
		if (this.state.phase === "idle") return;
		this.end(null);
		void this.engine.cancel().catch(() => {});
	}

	clearError(): void {
		if (this.state.error !== null) this.set({ error: null });
	}

	private async listen(run: number): Promise<void> {
		const session = ++this.session;
		this.part = "";
		let started: boolean;
		const opening = this.engine.start((event) => {
			if (run === this.run && session === this.session) this.heard(run, event);
		});
		this.opening = opening.catch(() => {});
		try {
			started = await opening;
		} catch (error) {
			if (run === this.run) this.fail(error instanceof Error ? error.message : DICTATION_NOT_STARTED);
			return;
		}
		if (run !== this.run) return;
		if (!started) return this.fail(DICTATION_NOT_STARTED);
		if (this.state.phase === "starting") this.set({ phase: "listening" });
	}

	private heard(run: number, event: DictationEvent): void {
		switch (event.type) {
			case "level":
				this.level(event.level);
				return;
			case "partial":
			case "final":
				this.part = event.text;
				this.show();
				return;
			case "error":
				this.fail(event.message || DICTATION_STOPPED);
				return;
			case "ended":
				if (event.reason === "cancelled") return;
				if (event.reason === "error") return this.fail(DICTATION_STOPPED);
				// The engine took a pause for the end: keep its words and listen on.
				this.committed.push(this.part);
				void this.listen(run);
				return;
		}
	}

	private show(): void {
		if (this.before === null) return;
		this.field.write(joinDictated(this.before, [...this.committed, this.part]));
	}

	/** Stopped on its own: the words heard stay in the field, and a sentence says why. */
	private fail(message: string): void {
		this.end(message);
		void this.engine.cancel().catch(() => {});
	}

	private end(error: string | null): void {
		this.run++;
		this.session++;
		this.before = null;
		this.committed = [];
		this.part = "";
		if (current === this) current = null;
		this.level(0);
		this.set({ phase: "idle", error });
	}

	private level(level: number): void {
		for (const listener of this.levels) listener(level);
	}

	private set(change: Partial<DictationView>): void {
		this.state = { ...this.state, ...change };
		for (const listener of this.listeners) listener();
	}
}

/**
 * This Mac's engine or the desk's, chosen again for every session by
 * `preferDesk`, so a change of hearing in Settings takes effect at the next
 * dictation. Each engine is asked for its permission the first time it is
 * chosen, since the controller asks the pair only once.
 */
export function eitherEngine(mac: DictationEngine, desk: DictationEngine, preferDesk: () => boolean): DictationEngine {
	let current = mac;
	const permitted = new WeakSet<DictationEngine>();
	const choose = async () => (preferDesk() && (await desk.capability()).available ? desk : mac);
	return {
		capability: async () => ((await mac.capability()).available ? { available: true } : desk.capability()),
		permit: async () => true,
		async start(onEvent) {
			const chosen = await choose();
			if (!permitted.has(chosen)) {
				if (!(await chosen.permit())) throw new Error(chosen === mac ? DICTATION_DENIED : DICTATION_NOT_STARTED);
				permitted.add(chosen);
			}
			current = chosen;
			return chosen.start(onEvent);
		},
		stop: () => current.stop(),
		cancel: () => current.cancel(),
	};
}

/** The window's one engine for dictation: the desk's, and on a Mac that hears, the Mac's beside it. */
let engine: DictationEngine | null = null;
export function dictationEngine(): DictationEngine {
	if (engine === null) {
		const speech = deviceTranscription();
		const desk = deskEngine(levelFromDbfs);
		engine = speech === undefined ? desk : eitherEngine(macEngine(speech), desk, () => !hearOnThisMac() && deskHears());
	}
	return engine;
}

/** The answer once it came, so a composer mounted later draws its key right the first time. */
let hears: boolean | undefined;

/** Whether this window can dictate: this Mac hears, or the desk has a model; false until asked. */
export function useDictationAvailable(): boolean {
	const desk = useDeskHears();
	const [mac, setMac] = useState(hears ?? false);
	useEffect(() => {
		let live = true;
		void hearsOnThisMac().then((here) => {
			hears = here;
			if (live) setMac(here);
		});
		return () => {
			live = false;
		};
	}, []);
	return mac || desk;
}

// ------------------------------------------------------------- tap or hold

/** A press held this long is talking while held; a shorter one is a tap that starts or stops. */
export const HOLD_MS = 300;
/** A press after this long without one is a new press, so a release the system lost cannot wedge the key. */
const REPEAT_GAP_MS = 2_500;

export function pressKind(heldMs: number): "tap" | "hold" {
	return heldMs >= HOLD_MS ? "hold" : "tap";
}

/**
 * One key or button that dictates both ways: a press starts listening at
 * once (or stops a dictation a tap left running), and its release stops
 * it only when the press was a hold. A press again before the release is
 * the key repeating, not a new press.
 */
export class TapOrHold {
	private downAt: number | null = null;
	private lastDownAt = 0;
	private started = false;

	/** The key went down. `listening` is whether a dictation is already on. */
	down(at: number, listening: boolean): "start" | "stop" | null {
		const repeat = this.downAt !== null && at - this.lastDownAt < REPEAT_GAP_MS;
		this.lastDownAt = at;
		if (repeat) return null;
		this.downAt = at;
		this.started = !listening;
		return listening ? "stop" : "start";
	}

	/** The key came up: a hold that started listening stops it. */
	up(at: number): "stop" | null {
		if (this.downAt === null) return null;
		const held = at - this.downAt;
		this.downAt = null;
		return this.started && pressKind(held) === "hold" ? "stop" : null;
	}
}

// ------------------------------------------------------------- sending after

/** The field is sent this long after a dictation stops, when the person asked for that. */
export const SEND_AFTER_MS = 1_500;

/** Fewer than two letters is a cough or a click, not a message. */
export function worthSending(heard: string): boolean {
	return heard.replace(/\s/g, "").length >= 2;
}

/**
 * The count down to sending what was dictated. It starts when a dictation
 * stops with words; the person can send at once, or call it off, which
 * leaves the words in the field. The clock is handed in, so tests need no
 * timers.
 */
export class SendCountdown {
	private cancelTimer: (() => void) | null = null;
	private startedAt: number | null = null;
	private readonly listeners = new Set<() => void>();

	constructor(
		private readonly send: () => void,
		private readonly after: (callback: () => void, ms: number) => () => void = (callback, ms) => {
			const timer = setTimeout(callback, ms);
			return () => clearTimeout(timer);
		},
	) {}

	/** When it began, while it counts; null otherwise. */
	get counting(): number | null {
		return this.startedAt;
	}

	readonly watch = (listener: () => void): (() => void) => {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	};

	/** False, and nothing counts, for words not worth sending. */
	start(heard: string, now = Date.now()): boolean {
		this.cancel();
		if (!worthSending(heard)) return false;
		this.startedAt = now;
		this.cancelTimer = this.after(() => this.fire(), SEND_AFTER_MS);
		this.changed();
		return true;
	}

	/** Escape, a key typed or a click in the field: the words stay, unsent. */
	cancel(): void {
		if (this.startedAt === null) return;
		this.cancelTimer?.();
		this.cancelTimer = null;
		this.startedAt = null;
		this.changed();
	}

	/** Enter: send now rather than in a moment. False when nothing was counting. */
	sendNow(): boolean {
		if (this.startedAt === null) return false;
		this.fire();
		return true;
	}

	private fire(): void {
		this.cancelTimer?.();
		this.cancelTimer = null;
		this.startedAt = null;
		this.changed();
		this.send();
	}

	private changed(): void {
		for (const listener of this.listeners) listener();
	}
}

/** What happens to dictated words when the person stops: this computer's choice, like its shortcuts. */
export type AfterDictation = "leave" | "send";
const AFTER_KEY = "hotline.dictation.after";
const afterListeners = new Set<() => void>();

function storedAfter(): AfterDictation {
	try {
		return localStorage.getItem(AFTER_KEY) === "send" ? "send" : "leave";
	} catch {
		return "leave";
	}
}
let after: AfterDictation = storedAfter();

export function afterDictation(): AfterDictation {
	return after;
}

export function setAfterDictation(next: AfterDictation): void {
	after = next;
	try {
		if (next === "leave") localStorage.removeItem(AFTER_KEY);
		else localStorage.setItem(AFTER_KEY, next);
	} catch {
		// Private mode: the choice holds until the app quits.
	}
	for (const listener of afterListeners) listener();
}

export function useAfterDictation(): AfterDictation {
	return useSyncExternalStore(
		(listener) => {
			afterListeners.add(listener);
			return () => afterListeners.delete(listener);
		},
		() => after,
	);
}

// ------------------------------------------------------------- the shortcut

/**
 * The Dictate shortcut's way to the conversation's composer: each press
 * and release, with when it happened, so the composer can tell a tap from
 * a hold. Edges with no composer on screen (Settings was open) wait
 * briefly for the one the window is about to show, which takes them as it
 * mounts.
 */
export type KeyEdge = "down" | "up";
const REQUEST_PATIENCE_MS = 3_000;
let target: ((edge: KeyEdge, at: number) => void) | null = null;
let waiting: { edge: KeyEdge; at: number }[] = [];

export function requestDictation(edge: KeyEdge, at = Date.now()): void {
	if (target !== null) {
		target(edge, at);
		return;
	}
	waiting.push({ edge, at });
}

/** The conversation's composer takes the shortcut's edges while it is mounted. */
export function takeDictationRequests(handle: (edge: KeyEdge, at: number) => void, now = Date.now()): () => void {
	target = handle;
	const fresh = waiting.filter((one) => now - one.at < REQUEST_PATIENCE_MS);
	waiting = [];
	// After the mount has settled: React may take a fresh mount down and up
	// once more (StrictMode), and that would let a dictation begun here go.
	if (fresh.length > 0) queueMicrotask(() => fresh.forEach((one) => handle(one.edge, one.at)));
	return () => {
		if (target === handle) target = null;
	};
}
