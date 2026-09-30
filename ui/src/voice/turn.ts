// Shared with Hotline Mobile: the phone copies this file. Keep it free of imports and of the DOM.
/**
 * Where one spoken turn starts and ends, from the microphone's level alone.
 *
 * The client meters the mic, hands each level to `push`, and records a clip
 * the whole time. `start` says the person is talking, `end` says the clip is
 * a turn to send, and `drop` says the clip so far is only room noise and is
 * to be thrown away. The values are Spark's, tuned by ear: 850ms of silence
 * was too slow to end a turn, and shorter cuts people off.
 */

/** A pause this long ends a turn. */
export const SILENCE_MS = 700;
/** Voice shorter than this is a cough or a tap, not a sentence. */
export const MIN_SPEECH_MS = 250;
/** The desk takes at most 20s per clip, so a turn is cut here. */
export const MAX_TURN_MS = 20_000;
/** A clip with no speech in it is dropped after this long, so silence never piles up. */
export const IDLE_DROP_MS = 8_000;

const INITIAL_FLOOR = 0.008;
/** Speech must clear this absolute level whatever the floor says. */
const MIN_VOICED_LEVEL = 0.018;
/** Speech must also clear the room's noise by this factor. */
const FLOOR_MARGIN = 3.2;
/** The floor only follows levels this close to it; anything louder is not the room. */
const FLOOR_FOLLOWS_BELOW = 2;
/** The floor moves 2% toward the level per frame at 60 frames a second. */
const FLOOR_STEP = 0.02;
const FRAME_MS = 1000 / 60;

export type TurnEvent =
	| { kind: "start"; at: number }
	| { kind: "end"; at: number; reason: "silence" | "max" }
	| { kind: "drop"; at: number; reason: "idle" };

const NONE: readonly TurnEvent[] = [];

/**
 * A metered level from decibels relative to full scale, as a phone's
 * recorder reports it, to the linear 0 to 1 that `push` takes.
 */
export function levelFromDb(db: number): number {
	return Number.isFinite(db) ? Math.min(1, 10 ** (db / 20)) : 0;
}

/**
 * One call's listener. Feed it the level (linear RMS, 0 to 1) and a
 * millisecond clock as often as the meter reports; it does not care how
 * often. After an `end` it ignores everything until `reset`, which is what
 * the client calls when it starts listening again.
 */
export class TurnDetector {
	private floor = INITIAL_FLOOR;
	private armed = true;
	private clipStart: number;
	private lastSample: number;
	private speechStart: number | null = null;
	private lastVoice = 0;
	private started = false;

	constructor(now: number) {
		this.clipStart = now;
		this.lastSample = now;
	}

	/** The room's noise level as far as it has been learned. */
	get noiseFloor(): number {
		return this.floor;
	}

	/** A fresh clip begins at `now`; the learned noise floor carries over. */
	reset(now: number): void {
		this.armed = true;
		this.clipStart = now;
		this.lastSample = now;
		this.speechStart = null;
		this.lastVoice = 0;
		this.started = false;
	}

	push(level: number, now: number): readonly TurnEvent[] {
		if (!this.armed) return NONE;
		const sample = Number.isFinite(level) && level > 0 ? level : 0;

		// Learn the room's noise, by the same weight per second whether the
		// meter reports sixty times a second or ten.
		if (sample < this.floor * FLOOR_FOLLOWS_BELOW) {
			const keep = (1 - FLOOR_STEP) ** (Math.max(0, now - this.lastSample) / FRAME_MS);
			this.floor = this.floor * keep + sample * (1 - keep);
		}
		this.lastSample = now;

		const voiced = sample > Math.max(MIN_VOICED_LEVEL, this.floor * FLOOR_MARGIN);
		const events: TurnEvent[] = [];
		if (voiced) {
			this.speechStart ??= now;
			this.lastVoice = now;
			if (!this.started && now - this.speechStart > MIN_SPEECH_MS) {
				this.started = true;
				events.push({ kind: "start", at: this.speechStart });
			}
		}

		const spoke = this.speechStart !== null && this.lastVoice - this.speechStart > MIN_SPEECH_MS;
		if (spoke && now - this.lastVoice > SILENCE_MS) {
			this.armed = false;
			events.push({ kind: "end", at: now, reason: "silence" });
		} else if (spoke && now - this.clipStart > MAX_TURN_MS) {
			this.armed = false;
			events.push({ kind: "end", at: now, reason: "max" });
		} else if (!spoke && this.speechStart === null && now - this.clipStart > IDLE_DROP_MS) {
			// Not while someone is mid-word: a clip that has just heard voice is not silence.
			this.reset(now);
			events.push({ kind: "drop", at: now, reason: "idle" });
		} else if (!voiced && !spoke && this.speechStart !== null && now - this.lastVoice > SILENCE_MS) {
			this.speechStart = null;
		}
		return events.length ? events : NONE;
	}
}

// Only a whole utterance counts ("OK bye", "bye bye, Hotline", "see you later"),
// so "bye" in the middle of a request never ends a call.
const GOODBYE =
	/^(?:(?:ok(?:ay)?|alright|all right|thanks|thank you)[ ,]+)*(?:bye(?:[ -]bye)?|goodbye|see (?:you|ya) later)(?:[ ,]+(?:hotline|desk))?$/;

/** Whether a transcript is nothing but a farewell, for the client to show the call ending. */
export function isGoodbye(transcript: string): boolean {
	const spoken = transcript
		.toLowerCase()
		.replace(/[^a-z ,-]/g, "")
		.replace(/\s+/g, " ")
		.trim();
	return GOODBYE.test(spoken);
}
