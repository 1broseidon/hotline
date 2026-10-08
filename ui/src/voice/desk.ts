import { useEffect, useSyncExternalStore } from "react";
import type { SpeechModel } from "../generated/contract";
import { wire } from "../wire";
import { webAudio } from "./audio";
import type { DictationEngine, DictationEvent } from "./dictation";
import { downsample, encodeWav, rms, toBase64, WAV_RATE } from "./wav";

/**
 * Dictation heard by the desk's own speech model (voice.md, Hearing on the
 * desk), for a window whose machine has no engine of its own, Windows and
 * Linux, or a Mac whose person picked the desk. The window keeps the
 * microphone: it meters each block for the VoiceMeter and gathers the
 * samples at 16 kHz, and the desk hears them with `voice.transcribe`. The
 * desk's model hears a whole clip, so words while talking are the clip so
 * far, heard again about once a second while no answer is outstanding.
 * A session ends itself after half a minute, the way a Mac's engine ends
 * one on a pause, so no clip outgrows what the desk takes and the
 * controller carries on in a fresh one.
 */

/** How often the words so far are asked for, at most. */
export const PARTIAL_EVERY_MS = 1_000;
/** A session's longest clip, half of what the desk takes at a time. */
export const SESSION_SECONDS = 30;
/** How long the desk has to hear the last of it once the person stops. */
const FINAL_PATIENCE_MS = 20_000;

export const DESK_MICROPHONE_DENIED = "Hotline can't use the microphone. Allow it for Hotline, then try again.";
const DESK_LOST_MICROPHONE = "The microphone went away. What was heard is in the field.";
const DESK_DID_NOT_FINISH = "The desk did not finish hearing that. Try again.";

/** The microphone as dictation needs it: blocks of samples, until closed. */
export type Microphone = {
	/** Rejects when the microphone cannot be had. */
	open(onBlock: (block: Float32Array, rate: number) => void, onLost: () => void): Promise<void>;
	close(): void;
};

/** What the desk engine needs from the world, so a test can be the world. */
export type DeskSeams = {
	/** Whether the desk has a model to hear with. */
	available(): Promise<boolean>;
	/** The words in one 16 kHz mono WAV, as standard base64. */
	transcribe(wav: string): Promise<string>;
	microphone(): Microphone;
	every(callback: () => void, ms: number): () => void;
	after(callback: () => void, ms: number): () => void;
};

function webMicrophone(): Microphone {
	let audio: ReturnType<typeof webAudio> | null = null;
	return {
		async open(onBlock, onLost) {
			audio = webAudio({ onIdle() {}, onLost });
			await audio.open(onBlock);
			await audio.openMic();
		},
		close() {
			audio?.close();
			audio = null;
		},
	};
}

const windowSeams: DeskSeams = {
	available: () => refreshDeskHearing(),
	transcribe: async (data) => (await wire.command("voice.transcribe", { mimeType: "audio/wav", data })).text,
	microphone: webMicrophone,
	every: (callback, ms) => {
		const timer = setInterval(callback, ms);
		return () => clearInterval(timer);
	},
	after: (callback, ms) => {
		const timer = setTimeout(callback, ms);
		return () => clearTimeout(timer);
	},
};

/** Loudness as the meter reads it: the block's RMS in dBFS, spread over 0 to 1 by `levelFromDbfs`. */
export function blockDbfs(block: Float32Array): number {
	const level = rms(block);
	return level > 0 ? 20 * Math.log10(level) : -Infinity;
}

export function deskEngine(level: (db: number) => number, seams: DeskSeams = windowSeams): DictationEngine {
	/** Bumped by every start and cancel, so a late answer of an old session is dropped. */
	let generation = 0;
	let mic: Microphone | null = null;
	let stopPartials: (() => void) | null = null;
	let chunks: Float32Array[] = [];
	let length = 0;
	/** How much of the session the last asked-for words covered. */
	let asked = 0;
	let asking = false;
	/** The words of a session that ended itself, while the desk is still hearing them. */
	let ending: Promise<string> | null = null;

	const clip = () => {
		const samples = new Float32Array(length);
		let at = 0;
		for (const chunk of chunks) {
			samples.set(chunk, at);
			at += chunk.length;
		}
		return toBase64(encodeWav(samples, WAV_RATE));
	};

	const release = () => {
		stopPartials?.();
		stopPartials = null;
		mic?.close();
		mic = null;
	};

	return {
		capability: async () => ({ available: await seams.available().catch(() => false) }),
		// The microphone is asked for when it opens; a refusal is the start's error.
		permit: async () => true,
		async start(onEvent: (event: DictationEvent) => void) {
			release();
			const token = ++generation;
			chunks = [];
			length = 0;
			asked = 0;
			asking = false;
			ending = null;
			const current = () => token === generation;

			const partial = () => {
				if (!current() || asking || length === asked || mic === null) return;
				asking = true;
				asked = length;
				seams
					.transcribe(clip())
					.then((text) => {
						if (current() && mic !== null) onEvent({ type: "partial", text });
					})
					// The final answer says whatever went wrong; a partial is a preview.
					.catch(() => {})
					.finally(() => {
						if (current()) asking = false;
					});
			};

			// Half a minute heard: this session ends with its words, as a pause would.
			const full = () => {
				release();
				ending = seams.transcribe(clip());
				ending.then(
					(text) => {
						if (!current()) return;
						onEvent({ type: "final", text });
						onEvent({ type: "ended", reason: "final" });
					},
					(error: unknown) => {
						if (current()) onEvent({ type: "error", message: error instanceof Error ? error.message : String(error) });
					},
				);
			};

			const next = seams.microphone();
			mic = next;
			try {
				await next.open(
					(block, rate) => {
						if (!current() || mic === null) return;
						const heard = downsample(block, rate);
						chunks.push(heard);
						length += heard.length;
						onEvent({ type: "level", level: level(blockDbfs(block)) });
						if (length >= SESSION_SECONDS * WAV_RATE) full();
					},
					() => {
						if (current()) onEvent({ type: "error", message: DESK_LOST_MICROPHONE });
					},
				);
			} catch {
				if (mic === next) release();
				if (!current()) return false;
				throw new Error(DESK_MICROPHONE_DENIED);
			}
			if (!current()) {
				next.close();
				return false;
			}
			stopPartials = seams.every(partial, PARTIAL_EVERY_MS);
			return true;
		},
		async stop() {
			const token = generation;
			// A session that ended itself is still being heard: its words are the answer.
			const hearing = mic === null ? ending : length === 0 ? null : seams.transcribe(clip());
			release();
			if (hearing === null) return "";
			let cancelTimer = () => {};
			try {
				const text = await Promise.race([
					hearing,
					new Promise<never>((_, reject) => {
						cancelTimer = seams.after(() => reject(new Error(DESK_DID_NOT_FINISH)), FINAL_PATIENCE_MS);
					}),
				]);
				return token === generation ? text.trim() : "";
			} finally {
				cancelTimer();
			}
		},
		async cancel() {
			generation++;
			release();
		},
	};
}

// ------------------------------------------------------------ whether the desk hears

/** Whether the open desk has a model installed, as last heard; false until asked. */
let deskHearing = false;
const listeners = new Set<() => void>();

function setDeskHearing(next: boolean): void {
	if (next === deskHearing) return;
	deskHearing = next;
	for (const listener of listeners) listener();
}

export function deskHears(): boolean {
	return deskHearing;
}

/** What Settings just heard of the models, so dictation knows without asking again. */
export function noteDeskModels(models: readonly SpeechModel[]): void {
	setDeskHearing(models.some((model) => model.state === "installed"));
}

/** Asks the desk; a desk too old to have models has none. */
export async function refreshDeskHearing(): Promise<boolean> {
	try {
		noteDeskModels(await wire.command("voice.models", {}));
	} catch {
		setDeskHearing(false);
	}
	return deskHearing;
}

/** Whether the desk can hear dictation, asked again whenever a component that wants to know mounts. */
export function useDeskHears(): boolean {
	useEffect(() => {
		void refreshDeskHearing();
	}, []);
	return useSyncExternalStore(
		(listener) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		deskHears,
		() => false,
	);
}
