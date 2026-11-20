/**
 * Interrupting the teammate by talking over it, on a Turn by turn call heard
 * by this Mac (docs/design/voice-interrupt.md). Energy alone never cuts a
 * reply: the voice-processed microphone above the playback's own floor only
 * pauses it, which is cheap and undone by resuming, and only words the reply
 * does not explain make it an interruption. The engine transcribes a reply
 * leaking into the microphone word for word even 40 dB down, so words that
 * are the reply's own are echo, never the person.
 *
 * Pure: the call feeds it levels, the output's loudness and the engine's
 * words, and does what it answers.
 */

/** Speech must clear this absolute level (linear, raw-equivalent) whatever else is quiet. */
const MIN_VOICED_LEVEL = 0.018;
/** Speech must also clear the room's noise, as the turn detector learned it, by this factor. */
const FLOOR_MARGIN = 3.2;
/** And the reply's residual echo, as learned from the playback, by this factor (about 10 dB). */
const ECHO_MARGIN = 3;
/** Output quieter than this teaches nothing about the echo. */
const AUDIBLE_OUTPUT = 0.01;
/** The output is compared at its loudest over this long: a headset's echo comes back late. */
const OUTPUT_SPAN_MS = 300;
/** How much playback the echo's coupling is learned from. */
const COUPLING_SPAN_MS = 2_000;
/**
 * The coupling is the median of the recent ratios of level to output: echo
 * keeps one ratio, and the person's first fifth of a second, a few readings
 * in forty, does not move a median.
 */
const COUPLING_RANK = 0.5;
/**
 * Playback heard before an onset may count: until the echo's coupling has
 * been learned, the reply's own start could be taken for the person.
 */
const WARMUP_MS = 400;
/** Voiced time that pauses the reply, within the window below. */
const ONSET_MS = 200;
const ONSET_WINDOW_MS = 400;
/** A pause lasts at least this long, so a word's first syllable is not resumed over. */
const MIN_PAUSE_MS = 500;
/** Quiet this long after the voice resumes the reply. */
const QUIET_MS = 500;
/** However long the person goes on without words, the reply resumes after this. */
export const MAX_PAUSE_MS = 4_000;
/**
 * Words count for this long after an onset, even once the reply resumed:
 * the engine gives words about once a second, and measured a sentence's
 * first words 2.6 s after they began over a playing reply.
 */
export const CONFIRM_MS = 3_000;
/** Onsets whose only words were the reply's own before the call stops listening over it. */
const ECHO_LIMIT = 2;

/** Listening sounds: said over someone talking, they ask nothing of them. */
const BACKCHANNEL = new Set([
	"ah", "aha", "cool", "eh", "er", "erm", "good", "great", "hm", "hmm", "huh", "mhm", "mm", "mmhmm",
	"nice", "oh", "ok", "okay", "right", "sure", "uh", "uhhuh", "um", "yeah", "yep", "yes", "yup",
]);
/** One of these is enough: they are what a person says to cut in. */
const CUT_WORDS = new Set(["actually", "hang", "hold", "hey", "no", "nope", "sorry", "stop", "wait"]);

export type BargeDecision =
	| { kind: "pause"; why: string }
	| { kind: "resume"; why: string }
	| { kind: "interrupt"; why: string }
	| { kind: "echo"; why: string }
	| { kind: "off"; why: string };

/** A word as compared: lower case, letters and digits only. */
function normal(word: string): string {
	return word.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
}

function wordsOf(text: string): string[] {
	return text.split(/\s+/).map(normal).filter((word) => word !== "");
}

/** Whether `word` is one the reply said, allowing the engine's tense and plural slips ("look" for "looked"). */
function sameWord(heard: string, said: string): boolean {
	if (heard === said) return true;
	return heard.length >= 3 && said.length >= 3 && (said.startsWith(heard) || heard.startsWith(said));
}

/** The reply's words in order, to tell its echo from the person. */
export function scriptOf(lines: readonly string[]): string[] {
	return lines.flatMap(wordsOf);
}

/** Echo is the reply heard back in order: this many of its words in a row, and fewer may be the person's own. */
const ECHO_RUN = 3;

/** How many of `words`, from the first, are a run of the reply's words in order. */
function echoRun(words: readonly string[], script: readonly string[]): number {
	let longest = 0;
	for (let start = 0; start < script.length; start++) {
		let length = 0;
		while (length < words.length && start + length < script.length && sameWord(words[length]!, script[start + length]!)) length++;
		longest = Math.max(longest, length);
	}
	return longest >= ECHO_RUN || (longest > 0 && longest === words.length) ? longest : 0;
}

/**
 * The person's words in `text`, the engine's whole utterance so far: what
 * follows `before` (the engine's words when the onset came, which were the
 * reply's echo or the room) and then any run of the reply's own words in
 * order, which the engine heard from before the reply paused. A word the
 * engine re-spelt since ("look" for "looked") still counts as `before`.
 */
export function personWords(text: string, before: string, script: readonly string[]): string {
	const words = text.trim().split(/\s+/).filter((word) => word !== "");
	const earlier = wordsOf(before);
	let first = earlier.length > 0 && earlier.every((word, index) => sameWord(normal(words[index] ?? ""), word)) ? earlier.length : 0;
	first += echoRun(words.slice(first).map(normal), script);
	return words.slice(first).join(" ");
}

export class BargeIn {
	private floor: number;
	private outputs: { at: number; level: number }[] = [];
	private ratios: { at: number; ratio: number }[] = [];
	private voiced: { from: number; to: number }[] = [];
	private lastSample: number;
	/** Audible playback the coupling has been learned from. */
	private learnedMs = 0;
	private previousVoiced = false;
	private pausedAt: number | null = null;
	private onset: number | null = null;
	private lastVoice = 0;
	/** The engine's words when the onset came: none of them are the person's. */
	private before = "";
	private echoes = 0;
	/** This onset's words so far were all the reply's own; counted once the onset is over. */
	private echoed = false;
	/** A voice the reply already resumed over, for want of words, pauses nothing until it stops. */
	private waitForQuiet = false;
	private stopped = false;

	constructor(now: number, noiseFloor: number) {
		this.floor = noiseFloor;
		this.lastSample = now;
	}

	get paused(): boolean {
		return this.pausedAt !== null;
	}

	/** When the voice that paused the reply began, and when it was last heard. */
	get voice(): { onset: number; last: number } | null {
		return this.onset === null ? null : { onset: this.onset, last: this.lastVoice };
	}

	/** The echo's coupling to the output: the residual level per unit of output, as recently heard. */
	private coupling(): number {
		if (this.ratios.length < 5) return 0;
		const sorted = this.ratios.map((one) => one.ratio).sort((a, b) => a - b);
		return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * COUPLING_RANK))]!;
	}

	/** What a level must clear to be the person, with `output` the reply's loudness now. */
	threshold(output: number): number {
		return Math.max(MIN_VOICED_LEVEL, this.floor * FLOOR_MARGIN, this.coupling() * output * ECHO_MARGIN);
	}

	/**
	 * One meter reading of the processed microphone, `level`, while the
	 * reply plays at `output` (0 while paused), and the engine's words so far.
	 */
	level(level: number, output: number, now: number, text: string): BargeDecision | null {
		if (this.stopped) return null;
		const elapsed = Math.max(0, now - this.lastSample);
		this.lastSample = now;
		this.outputs.push({ at: now, level: output });
		while (this.outputs[0] !== undefined && now - this.outputs[0].at > OUTPUT_SPAN_MS) this.outputs.shift();
		const loudest = this.outputs.reduce((most, one) => Math.max(most, one.level), 0);
		const voiced = level > this.threshold(loudest);
		// Learn the echo only from playback nobody has yet been heard over.
		if (this.pausedAt === null && this.onset === null && loudest >= AUDIBLE_OUTPUT) {
			this.ratios.push({ at: now, ratio: level / loudest });
			while (this.ratios[0] !== undefined && now - this.ratios[0].at > COUPLING_SPAN_MS) this.ratios.shift();
			if (elapsed <= 150) this.learnedMs += elapsed;
		}
		// No onset over playback whose echo is not yet known; with nothing playing there is no echo.
		const listening = this.learnedMs >= WARMUP_MS || loudest < AUDIBLE_OUTPUT;
		if (voiced) this.lastVoice = now;
		if (this.waitForQuiet && now - this.lastVoice >= QUIET_MS) this.waitForQuiet = false;
		if (this.pausedAt === null) {
			const from = now - ONSET_WINDOW_MS;
			while (this.voiced[0] !== undefined && this.voiced[0].to <= from) this.voiced.shift();
			if (elapsed > 0 && elapsed <= 150 && (voiced || this.previousVoiced)) {
				const middle = now - elapsed / 2;
				this.voiced.push({ from: this.previousVoiced ? now - elapsed : middle, to: voiced ? now : middle });
			}
			this.previousVoiced = voiced;
			const voicedMs = this.voiced.reduce((total, one) => total + one.to - Math.max(from, one.from), 0);
			if (voiced && listening && !this.waitForQuiet && voicedMs >= ONSET_MS) {
				this.pausedAt = now;
				this.onset = Math.max(from, this.voiced[0]!.from);
				this.before = text;
				this.echoed = false;
				this.voiced = [];
				return { kind: "pause", why: `voice ${level.toFixed(3)} over ${this.threshold(loudest).toFixed(3)}` };
			}
			// Past the window for words, the onset is forgotten.
			if (this.onset !== null && now - this.onset > CONFIRM_MS) {
				this.onset = null;
				return this.countEcho();
			}
			return null;
		}
		this.previousVoiced = voiced;
		if (now - this.pausedAt >= MAX_PAUSE_MS) {
			this.waitForQuiet = true;
			return this.resume(`no words in ${MAX_PAUSE_MS} ms`);
		}
		if (now - this.pausedAt >= MIN_PAUSE_MS && now - this.lastVoice >= QUIET_MS) return this.resume("quiet, no words");
		return null;
	}

	/** Resumes the reply; an onset whose words were only the reply's echo may also stop listening over it. */
	private resume(why: string): BargeDecision {
		this.pausedAt = null;
		this.voiced = [];
		const echo = this.countEcho();
		if (echo?.kind === "off") return echo;
		return { kind: "resume", why: echo === null ? why : `${why}, ${echo.why}` };
	}

	/** An onset over whose only words were the reply's own: its echo came through the canceller. */
	private countEcho(): BargeDecision | null {
		if (!this.echoed) return null;
		this.echoed = false;
		this.echoes++;
		if (this.echoes >= ECHO_LIMIT) {
			this.stopped = true;
			this.pausedAt = null;
			return { kind: "off", why: `the reply's own words were heard over it ${this.echoes} times` };
		}
		return { kind: "echo", why: "its only words were the reply's own" };
	}

	/**
	 * The engine's whole utterance so far, `text`, against `script`, the
	 * reply's words. Words count only within `CONFIRM_MS` of an onset.
	 */
	words(text: string, now: number, script: readonly string[]): BargeDecision | null {
		if (this.stopped || this.onset === null || now - this.onset > CONFIRM_MS) return null;
		const said = wordsOf(personWords(text, this.before, script));
		// Words the reply also said may be its echo: they never count towards a cut on their own.
		const asking = said.filter((word) => !BACKCHANNEL.has(word) && !script.some((one) => sameWord(word, one)));
		const cutWord = said.some((word) => CUT_WORDS.has(word));
		if (asking.length >= 2 || cutWord) {
			this.stopped = true;
			this.pausedAt = null;
			return { kind: "interrupt", why: cutWord ? "a word that cuts in" : `${asking.length} words the reply did not say` };
		}
		// Words, and every one of them the reply's own, may be its echo; it counts if no more come.
		const since = wordsOf(personWords(text, this.before, []));
		this.echoed = said.length === 0 && since.length > 0;
		return null;
	}

	/** The engine's words when the voice that paused the reply began. */
	get wordsBefore(): string {
		return this.before;
	}
}
