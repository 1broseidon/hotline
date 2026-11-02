import { useCallback, useEffect, useSyncExternalStore } from "react";

/** Where a card's answer stands: nothing out, or one on its way and the tape not yet showing it. */
export type AnswerState = { k: "idle"; failure?: string } | { k: "answering" };

const IDLE: AnswerState = { k: "idle" };

/** Longer than a command takes to land and the tape to show it; past it the card is let go. */
export const ANSWER_WATCHDOG_MS = 10_000;

const NOT_SEEN = "Hotline did not confirm that answer. Check whether the card is still waiting, then try again.";

export type Timers = { set(fn: () => void, ms: number): unknown; clear(id: unknown): void };

const realTimers: Timers = { set: (fn, ms) => setTimeout(fn, ms), clear: (id) => clearTimeout(id as ReturnType<typeof setTimeout>) };

/**
 * The answers on their way, one per card key, shared by every mount of a
 * card: the inline card and the composer's panel are the same card and see the
 * same state, so one click cannot be followed by a second. A card is idle,
 * then answering from the click until its status moves on the tape (`settle`),
 * its command is refused, or the watchdog finds the status never moved.
 */
export class Answers {
	private readonly states = new Map<string, AnswerState>();
	private readonly watchdogs = new Map<string, unknown>();
	private readonly listeners = new Set<() => void>();

	constructor(
		private readonly timers: Timers = realTimers,
		private readonly watchdogMs = ANSWER_WATCHDOG_MS,
	) {}

	get(key: string): AnswerState {
		return this.states.get(key) ?? IDLE;
	}

	subscribe = (listener: () => void): (() => void) => {
		this.listeners.add(listener);
		return () => void this.listeners.delete(listener);
	};

	/** Sends the answer unless one is already out. Returns whether it was sent. */
	begin(key: string, send: () => Promise<unknown>, describe: (error: unknown) => string = message): boolean {
		if (this.get(key).k === "answering") return false;
		this.put(key, { k: "answering" });
		this.watchdogs.set(
			key,
			this.timers.set(() => this.release(key, { k: "idle", failure: NOT_SEEN }), this.watchdogMs),
		);
		let sent: Promise<unknown>;
		try {
			sent = send();
		} catch (error) {
			this.release(key, { k: "idle", failure: describe(error) });
			return true;
		}
		// Success is the tape moving; only a refusal frees the card here.
		sent.catch((error: unknown) => this.release(key, { k: "idle", failure: describe(error) }));
		return true;
	}

	/** The card's status moved: whatever was out has landed. */
	settle(key: string): void {
		if (!this.states.has(key) && !this.watchdogs.has(key)) return;
		this.release(key, undefined);
	}

	private release(key: string, next: AnswerState | undefined): void {
		const dog = this.watchdogs.get(key);
		if (dog !== undefined) this.timers.clear(dog);
		this.watchdogs.delete(key);
		if (next === undefined) this.states.delete(key);
		else this.states.set(key, next);
		for (const listener of this.listeners) listener();
	}

	private put(key: string, state: AnswerState): void {
		this.states.set(key, state);
		for (const listener of this.listeners) listener();
	}
}

function message(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

/** The key one card's answering state is kept under, the same from every place it is drawn. */
export function askKey(event: { kind: string; id: string }): string {
	return `${event.kind}:${event.id}`;
}

/** The desk's answers: one store, so a card mounted twice is one card. */
export const answers = new Answers();

/**
 * A card's answering state, shared by every mount of it. `settled` is true
 * once the tape shows the card decided; that forgets the answer that was out.
 */
export function useAnswer(key: string, settled: boolean, describe?: (error: unknown) => string) {
	const state = useSyncExternalStore(answers.subscribe, () => answers.get(key), () => IDLE);
	useEffect(() => {
		if (settled) answers.settle(key);
	}, [key, settled]);
	const answer = useCallback(
		(send: () => Promise<unknown>) => !settled && answers.begin(key, send, describe),
		// biome-ignore lint/correctness/useExhaustiveDependencies: `describe` is a literal per card kind.
		[key, settled],
	);
	return { answering: state.k === "answering", failure: state.k === "idle" ? (state.failure ?? null) : null, answer };
}
