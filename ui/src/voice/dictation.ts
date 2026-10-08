import { useEffect, useState } from "react";
import { type DeviceTranscription, deviceTranscription, hearsOnThisMac } from "./transcription";

/**
 * Dictation into the composer: an engine hears the person and the words go
 * into the field, never to a teammate until the person sends them. The
 * engine works one utterance per session; a pause it takes for the end of
 * an utterance starts a fresh session and the words carry on after the last
 * ones, until the person stops. Kept out of React so the merging and the
 * engine's order of events can be tested without a window.
 *
 * Today the one engine is this Mac's (`macEngine`, over transcription.ts).
 * Any other source of words and loudness, such as one running in the desk
 * and reporting over the wire, plugs in as another `DictationEngine`; the
 * controller, the composer and the meter do not change.
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
		start: (onEvent) => speech.start((event) => onEvent(event.type === "level" ? { type: "level", level: levelFromDbfs(event.levelDb) } : event)),
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
};

export const DICTATION_UNAVAILABLE = "Dictation isn't available on this Mac.";
export const DICTATION_DENIED =
	"Hotline can't hear you. Allow Hotline under Speech Recognition and Microphone in System Settings › Privacy & Security, then try again.";
const DICTATION_NOT_STARTED = "Dictation couldn't start. Try again.";
const DICTATION_STOPPED = "Dictation stopped. What it heard is in the field.";

/** The microphone's loudness in dBFS from a quiet room to a raised voice, which a level spreads across 0 to 1. */
const QUIET_DB = -60;
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

export const NOT_DICTATING: DictationView = { phase: "idle", error: null };

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
		this.end(null);
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

/** The window's one engine for dictation, or none where this machine cannot hear. */
let engine: DictationEngine | undefined | null = null;
export function dictationEngine(): DictationEngine | undefined {
	if (engine === null) {
		const speech = deviceTranscription();
		engine = speech === undefined ? undefined : macEngine(speech);
	}
	return engine;
}

/** The answer once it came, so a composer mounted later draws its key right the first time. */
let hears: boolean | undefined;

/** Whether this machine can dictate; false until asked, and on any machine but a Mac that hears. */
export function useDictationAvailable(): boolean {
	const [available, setAvailable] = useState(hears ?? false);
	useEffect(() => {
		let live = true;
		void hearsOnThisMac().then((here) => {
			hears = here;
			if (live) setAvailable(here);
		});
		return () => {
			live = false;
		};
	}, []);
	return available;
}

/**
 * The shortcut's way to the conversation's composer. A press with no
 * composer on screen (Settings was open) waits briefly for the one the
 * window is about to show, which takes it as it mounts.
 */
const REQUEST_PATIENCE_MS = 3_000;
let target: (() => void) | null = null;
let requestedAt: number | null = null;

export function requestDictation(now = Date.now()): void {
	if (target !== null) {
		target();
		return;
	}
	requestedAt = now;
}

/** The conversation's composer takes the shortcut's presses while it is mounted. */
export function takeDictationRequests(toggle: () => void, now = Date.now()): () => void {
	target = toggle;
	const waiting = requestedAt;
	requestedAt = null;
	// After the mount has settled: React may take a fresh mount down and up
	// once more (StrictMode), and that would let a dictation begun here go.
	if (waiting !== null && now - waiting < REQUEST_PATIENCE_MS) queueMicrotask(toggle);
	return () => {
		if (target === toggle) target = null;
	};
}
