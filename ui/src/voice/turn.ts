/**
 * PLACEHOLDER until Wren's port of Spark's detector lands on voice/speech;
 * on merge, theirs wins and call.ts adapts to it. Same numbers as Spark:
 * 700ms of silence ends a turn, blips under 250ms are ignored, a turn is
 * capped at 20s, and a clip that is only silence is dropped after 8s.
 */
export const SILENCE_MS = 700;
export const MIN_SPEECH_MS = 250;
export const MAX_TURN_MS = 20_000;
export const IDLE_RESET_MS = 8_000;

export type TurnEvent =
	/** Speech has gone on long enough to be a turn. */
	| { kind: "hearing" }
	/** The turn is over: send what was recorded since the last reset. */
	| { kind: "end"; reason: "silence" | "max" }
	/** Nothing worth sending: throw the recording away and start again. */
	| { kind: "reset" }
	/** A blip, not a sentence: back to listening, recording kept. */
	| { kind: "listening" };

export class TurnDetector {
	private floor = 0.008;
	private speechStart = 0;
	private lastVoice = 0;
	private turnStart = 0;

	constructor(now: number) {
		this.turnStart = now;
	}

	reset(now: number): void {
		this.speechStart = 0;
		this.lastVoice = 0;
		this.turnStart = now;
	}

	/** One level reading (RMS, 0..1) at `now` ms. */
	push(level: number, now: number): TurnEvent | null {
		if (level < this.floor * 2) this.floor = this.floor * 0.98 + level * 0.02;
		const voiced = level > Math.max(0.018, this.floor * 3.2);
		let event: TurnEvent | null = null;
		if (voiced) {
			if (!this.speechStart) this.speechStart = now;
			const wasShort = this.lastVoice - this.speechStart <= MIN_SPEECH_MS;
			this.lastVoice = now;
			if (wasShort && now - this.speechStart > MIN_SPEECH_MS) event = { kind: "hearing" };
		}
		const spoke = this.speechStart > 0 && this.lastVoice - this.speechStart > MIN_SPEECH_MS;
		if (spoke && now - this.lastVoice > SILENCE_MS) return { kind: "end", reason: "silence" };
		if (spoke && now - this.turnStart > MAX_TURN_MS) return { kind: "end", reason: "max" };
		if (!spoke && now - this.turnStart > IDLE_RESET_MS) {
			this.reset(now);
			return { kind: "reset" };
		}
		if (!voiced && this.speechStart && !spoke && now - this.lastVoice > SILENCE_MS) {
			this.speechStart = 0;
			return { kind: "listening" };
		}
		return event;
	}
}

const GOODBYE = /^\s*(ok(ay)?[,.\s]+)?(thanks?( you)?[,.\s]+)?(bye|goodbye|good bye|bye bye|see you|talk (to you )?later|that'?s all|hang up)[\s.!]*$/i;

export function isGoodbye(text: string): boolean {
	return GOODBYE.test(text);
}
