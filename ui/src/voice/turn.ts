// Shared with Hotline Mobile: the phone copies this file. Keep it free of imports and of the DOM.
/**
 * Where one spoken turn starts and ends, from the microphone's level alone.
 *
 * The client meters the mic, hands each level to `push`, and records a clip
 * the whole time. `start` says the person is talking, `end` says the clip is
 * a turn to send, and `drop` says the clip so far is only room noise and is
 * to be thrown away (a `start` may have come first: a fan switching on sounds
 * like a person until the floor has caught up with it). The values are
 * Spark's, tuned by ear: 850ms of silence was too slow to end a turn, and
 * shorter cuts people off.
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
/**
 * How long the level has to hold still before it is the room and not a
 * person. Speech does not: it dips between words and syllables, and the
 * quietest moment in a second and a half of it is well under the loudest. A
 * fan, a hum or an air conditioner does not dip.
 */
const STEADY_MS = 1500;
/** Still enough: the loudest level in that time is at most this many times the quietest. */
const STEADY_RATIO = 1.7;
/** A steady level moves the floor 63% of the way to it in this long. */
const FLOOR_RISE_MS = 5000;

export type TurnEvent =
	| { kind: "start"; at: number }
	| { kind: "end"; at: number; reason: "silence" | "max" }
	| { kind: "drop"; at: number; reason: "idle" | "noise" };

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
	/** The loudest voiced level since this turn began. */
	private peak = 0;
	/** The last STEADY_MS of levels, to tell a room from a person. */
	private recent: { at: number; level: number }[] = [];

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
		this.peak = 0;
		this.recent = [];
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
		// A level that holds still is the room, however loud. The floor rises to
		// it slowly, so a fan that comes on is at first taken for a person and
		// then, once the floor has caught up, is not.
		this.recent.push({ at: now, level: sample });
		while (this.recent[0] !== undefined && now - this.recent[0].at > STEADY_MS) this.recent.shift();
		const steady = this.steadyLevel(now);
		if (steady !== null && steady > this.floor * FLOOR_FOLLOWS_BELOW) {
			const keep = Math.exp(-Math.max(0, now - this.lastSample) / FLOOR_RISE_MS);
			this.floor = this.floor * keep + steady * (1 - keep);
		}
		this.lastSample = now;

		const voiced = sample > this.threshold();
		const events: TurnEvent[] = [];
		if (voiced) {
			this.speechStart ??= now;
			this.lastVoice = now;
			this.peak = Math.max(this.peak, sample);
			if (!this.started && now - this.speechStart > MIN_SPEECH_MS) {
				this.started = true;
				events.push({ kind: "start", at: this.speechStart });
			}
		}

		const spoke = this.speechStart !== null && this.lastVoice - this.speechStart > MIN_SPEECH_MS;
		if (spoke && now - this.lastVoice > SILENCE_MS) {
			if (this.peak <= this.threshold()) {
				// The loudest it ever got is what the floor now calls the room.
				this.reset(now);
				events.push({ kind: "drop", at: now, reason: "noise" });
			} else {
				this.armed = false;
				events.push({ kind: "end", at: now, reason: "silence" });
			}
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

	/** What a level must clear to be voice. */
	private threshold(): number {
		return Math.max(MIN_VOICED_LEVEL, this.floor * FLOOR_MARGIN);
	}

	/** The average of the last STEADY_MS if it held still all that time, else null. */
	private steadyLevel(now: number): number | null {
		const oldest = this.recent[0];
		// A window that has not yet been STEADY_MS long has not held still for that long.
		if (oldest === undefined || this.recent.length < 3 || now - oldest.at < STEADY_MS * 0.9) return null;
		let low = Number.POSITIVE_INFINITY;
		let high = 0;
		let sum = 0;
		for (const { level } of this.recent) {
			low = Math.min(low, level);
			high = Math.max(high, level);
			sum += level;
		}
		return high <= low * STEADY_RATIO ? sum / this.recent.length : null;
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
