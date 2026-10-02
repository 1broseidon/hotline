// Shared with Hotline Mobile. Keep it free of imports and of the DOM.
/**
 * Where one spoken turn starts and ends, from the microphone's level alone.
 *
 * The client meters the mic, hands each level to `push`, and records a clip
 * the whole time. `start` says the person is talking, `end` says the clip is
 * a turn to send, and `drop` says the clip so far is only room noise and is
 * to be thrown away. A level alone cannot distinguish a long beep from a
 * voice. Short noises must contribute enough actual voiced time to start,
 * and an established voice gets a quieter continuation threshold and room
 * to pause. The noise floor is learned only before speech is confirmed.
 */

/** A pause this long ends a turn. */
export const SILENCE_MS = 1200;
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
/** Softer syllables keep a confirmed utterance alive without opening a new one. */
const CONTINUE_MARGIN = 1.6;
const MIN_CONTINUE_LEVEL = 0.012;
/** Onset needs MIN_SPEECH_MS of voiced time within this short window. */
const ONSET_WINDOW_MS = 500;
/** Longer meter gaps are unknown audio, never evidence of speech or silence. */
const MAX_SAMPLE_GAP_MS = 150;
/** The floor only follows levels this close to it; anything louder is not the room. */
const FLOOR_FOLLOWS_BELOW = 2;
/** The floor moves 2% toward the level per frame at 60 frames a second. */
const FLOOR_STEP = 0.02;
const FRAME_MS = 1000 / 60;
/**
 * An unconfirmed steady level can teach the room's floor. Once a voice is
 * confirmed, even a steady vowel must not be learned as background noise.
 */
const STEADY_MS = 1500;
/** Still enough: the loudest level in that time is at most this many times the quietest. */
const STEADY_RATIO = 1.7;
/** A steady level moves the floor 63% of the way to it in this long. */
const FLOOR_RISE_MS = 5000;

export type TurnEvent =
  | { kind: 'start'; at: number }
  | { kind: 'end'; at: number; reason: 'silence' | 'max' }
  | { kind: 'drop'; at: number; reason: 'idle' | 'noise' };

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
  private lastVoice = 0;
  private started = false;
  private sampled = false;
  private previousVoiced = false;
  /** Measured voiced intervals, with transitions estimated halfway between samples. */
  private onset: { from: number; to: number }[] = [];
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
    this.lastVoice = 0;
    this.started = false;
    this.sampled = false;
    this.previousVoiced = false;
    this.onset = [];
    this.recent = [];
  }

  push(level: number | null, now: number): readonly TurnEvent[] {
    if (!this.armed) return NONE;
    const elapsed = Math.max(0, now - this.lastSample);
    const available = level !== null && Number.isFinite(level) && level >= 0;
    const continuous = available && this.sampled && elapsed <= MAX_SAMPLE_GAP_MS;
    if (!continuous) {
      // Freeze the silence timer across unavailable audio. A recovered meter
      // gets a full measured pause; an outage cannot cut off the speaker.
      if (this.started) this.lastVoice += elapsed;
      this.onset = [];
      this.recent = [];
      this.previousVoiced = false;
    }
    const before = this.lastSample;
    this.lastSample = now;
    this.sampled = available;
    if (!available) return NONE;
    const sample = level!;
    const measured = continuous ? elapsed : 0;

    // Learn the room's noise, by the same weight per second whether the
    // meter reports sixty times a second or ten.
    if (!this.started) {
      if (sample < this.floor * FLOOR_FOLLOWS_BELOW) {
        const keep = (1 - FLOOR_STEP) ** (measured / FRAME_MS);
        this.floor = this.floor * keep + sample * (1 - keep);
      }
      this.recent.push({ at: now, level: sample });
      while (this.recent[0] !== undefined && now - this.recent[0].at > STEADY_MS)
        this.recent.shift();
      const steady = this.steadyLevel(now);
      if (steady !== null && steady > this.floor * FLOOR_FOLLOWS_BELOW) {
        const keep = Math.exp(-measured / FLOOR_RISE_MS);
        this.floor = this.floor * keep + steady * (1 - keep);
      }
    }

    const voiced = sample > this.threshold();
    const events: TurnEvent[] = [];
    if (!this.started) {
      const from = now - ONSET_WINDOW_MS;
      while (this.onset[0] !== undefined && this.onset[0].to <= from) this.onset.shift();
      if (measured > 0 && (voiced || this.previousVoiced)) {
        const middle = before + measured / 2;
        this.onset.push({
          from: this.previousVoiced ? before : middle,
          to: voiced ? now : middle,
        });
      }
      const voicedMs = this.onset.reduce(
        (total, interval) => total + interval.to - Math.max(from, interval.from),
        0,
      );
      if (voiced && voicedMs >= MIN_SPEECH_MS) {
        this.started = true;
        events.push({ kind: 'start', at: Math.max(from, this.onset[0]!.from) });
        this.onset = [];
        this.recent = [];
      }
    }
    this.previousVoiced = voiced;
    if (voiced && this.started) {
      this.lastVoice = now;
    }

    if (this.started && now - this.lastVoice > SILENCE_MS) {
      this.armed = false;
      events.push({ kind: 'end', at: now, reason: 'silence' });
    } else if (this.started && now - this.clipStart > MAX_TURN_MS) {
      this.armed = false;
      events.push({ kind: 'end', at: now, reason: 'max' });
    } else if (
      !this.started &&
      !voiced &&
      this.onset.length === 0 &&
      now - this.clipStart > IDLE_DROP_MS
    ) {
      // Not while someone is mid-word: a clip that has just heard voice is not silence.
      this.reset(now);
      events.push({ kind: 'drop', at: now, reason: 'idle' });
    }
    return events.length ? events : NONE;
  }

  /** What a level must clear to be voice. */
  private threshold(): number {
    return this.started
      ? Math.max(MIN_CONTINUE_LEVEL, this.floor * CONTINUE_MARGIN)
      : Math.max(MIN_VOICED_LEVEL, this.floor * FLOOR_MARGIN);
  }

  /** The average of the last STEADY_MS if it held still all that time, else null. */
  private steadyLevel(now: number): number | null {
    const oldest = this.recent[0];
    // A window that has not yet been STEADY_MS long has not held still for that long.
    if (oldest === undefined || this.recent.length < 3 || now - oldest.at < STEADY_MS * 0.9)
      return null;
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
    .replace(/[^a-z ,-]/g, '')
    .replace(/\s+/g, ' ')
    .trim();
  return GOODBYE.test(spoken);
}
