import { useEffect, useSyncExternalStore } from "react";
import type { Attachment, TranscriptEvent } from "./generated/contract";

/** The message a dropped socket rejects every waiting command with (wire.ts): the core may have run it. */
export const DROPPED = "The connection to Hotline dropped.";

const UNSURE = "The connection to Hotline dropped while that was sending. If it does not appear, send it again.";

/** How long a send the core may have taken stays on screen waiting to be seen. */
const UNSURE_MS = 60_000;

/** A quoted line a send answers: the id the core links it by and the words shown. */
type Reply = { eventId: string; text: string };

export type SendInput = { text: string; attachments: Attachment[]; reply?: Reply };

/**
 * A message on its way. `sending` has not been answered; `unsure` lost its
 * socket, so the core may or may not have it, and is kept on screen rather than
 * handed back, because handing it back invites a second copy.
 */
export type SendLine = SendInput & { id: number; at: number; phase: "sending" | "unsure" };

/** The words of a send the core refused, waiting to go back where they were typed. */
type Returned = SendInput & { id: number; message: string };

export type ConvSends = {
	lines: readonly SendLine[];
	returned: readonly Returned[];
	/** What to say in the band about the last send, until a send succeeds. */
	notice: string | null;
};

const NOTHING: ConvSends = { lines: [], returned: [], notice: null };

/** Refused sends gathered into the one thing the composer is handed back. */
export type Gathered = { text: string; attachments: Attachment[]; reply?: Reply; message: string; nonce: number };

/**
 * Sends in flight and refused, per conversation (a teammate's, or a thread's
 * `kind:key`), held outside any screen: a conversation that is switched away
 * from before its send settles still gets its words back, and one slot is never
 * shared between two sends. Within a conversation sends reach the core in the
 * order they were made, and settle each on their own.
 */
export class Sends {
	private readonly states = new Map<string, ConvSends>();
	private readonly tails = new Map<string, Promise<unknown>>();
	private readonly starts = new Map<string, Promise<unknown>>();
	private readonly listeners = new Set<() => void>();
	private next = 1;

	constructor(private readonly now: () => number = Date.now) {}

	get(conv: string): ConvSends {
		return this.states.get(conv) ?? NOTHING;
	}

	subscribe = (listener: () => void): (() => void) => {
		this.listeners.add(listener);
		return () => void this.listeners.delete(listener);
	};

	/** Puts the message on the conversation's line and sends it once those before it have settled. */
	send(conv: string, input: SendInput, run: () => Promise<unknown>): void {
		const line: SendLine = { ...input, id: this.next++, at: this.now(), phase: "sending" };
		const live = this.get(conv).lines.filter((one) => one.phase === "sending" || line.at - one.at < UNSURE_MS);
		this.put(conv, { ...this.get(conv), lines: [...live, line] });
		const done = (this.tails.get(conv) ?? Promise.resolve()).then(run);
		this.tails.set(conv, done.catch(() => {}));
		done.then(
			() => {
				this.put(conv, { ...this.get(conv), lines: this.get(conv).lines.filter((one) => one.id !== line.id), notice: null });
			},
			(error: unknown) => {
				const message = error instanceof Error ? error.message : String(error);
				const was = this.get(conv);
				if (message === DROPPED) {
					this.put(conv, { ...was, lines: was.lines.map((one) => (one.id === line.id ? { ...one, phase: "unsure" as const } : one)), notice: UNSURE });
					return;
				}
				const { id: _id, at: _at, phase: _phase, ...words } = line;
				this.put(conv, {
					lines: was.lines.filter((one) => one.id !== line.id),
					returned: [...was.returned, { ...words, id: line.id, message }],
					notice: message,
				});
			},
		);
	}

	/** Everything the conversation has been refused, as one thing to put back; nothing when there is none. */
	take(conv: string): Gathered | undefined {
		const was = this.get(conv);
		if (was.returned.length === 0) return undefined;
		const first = was.returned.find((one) => one.reply !== undefined)?.reply;
		const last = was.returned[was.returned.length - 1]!;
		this.put(conv, { ...was, returned: [] });
		return {
			text: was.returned.map((one) => one.text).filter((text) => text !== "").join("\n\n"),
			attachments: was.returned.flatMap((one) => one.attachments),
			...(first !== undefined ? { reply: first } : {}),
			message: last.message,
			nonce: last.id,
		};
	}

	/** The tape now holds these: a message kept on screen because its socket dropped has been seen. */
	landed(conv: string, events: readonly TranscriptEvent[]): void {
		const was = this.get(conv);
		const unsure = was.lines.filter((one) => one.phase === "unsure");
		if (unsure.length === 0) return;
		const seen = new Set(unlanded(unsure, events).map((one) => one.id));
		const gone = unsure.filter((one) => !seen.has(one.id));
		if (gone.length === 0) return;
		this.put(conv, { ...was, lines: was.lines.filter((one) => !gone.includes(one)) });
	}

	/** Runs `make` unless one run for this key is still out, and hands every caller that run: one session start, however many sends ask. */
	once(key: string, make: () => Promise<unknown>): Promise<unknown> {
		const running = this.starts.get(key);
		if (running !== undefined) return running;
		const mine = make().finally(() => {
			if (this.starts.get(key) === mine) this.starts.delete(key);
		});
		this.starts.set(key, mine);
		return mine;
	}

	private put(conv: string, state: ConvSends): void {
		this.states.set(conv, state);
		for (const listener of this.listeners) listener();
	}
}

/**
 * The lines still to show: each message the tape holds hides one line of the
 * same words sent at or after it (a second of slack for clocks), so the same
 * words sent twice show twice until both have landed.
 */
export function unlanded(lines: readonly SendLine[], events: readonly TranscriptEvent[]): SendLine[] {
	const used = new Set<string>();
	return [...lines]
		.sort((a, b) => a.at - b.at)
		.filter((line) => {
			const match = events.find((event) => event.kind === "user" && !used.has(event.id) && event.ts >= line.at - 1000 && event.text === line.text);
			if (match === undefined) return true;
			used.add(match.id);
			return false;
		});
}

/** The desk's sends: one store, so a screen that goes away leaves its sends where the next one finds them. */
export const sends = new Sends();

/** A conversation's sends, read live. */
export function useSends(conv: string): ConvSends {
	return useSyncExternalStore(sends.subscribe, () => sends.get(conv), () => NOTHING);
}

/**
 * Hands the conversation what it was refused as it arrives, and when it is
 * first shown: words sent, refused while another teammate was open, are back
 * in this one's field when it is.
 */
export function useReturned(conv: string, onReturned: (back: Gathered) => void): void {
	const { returned } = useSends(conv);
	const count = returned.length;
	// biome-ignore lint/correctness/useExhaustiveDependencies: a refusal arriving is what takes the words.
	useEffect(() => {
		if (count === 0) return;
		const back = sends.take(conv);
		if (back !== undefined) onReturned(back);
	}, [conv, count]);
}
