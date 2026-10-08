import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { platform } from "../native";

/**
 * Speech recognition on this Mac, as the phone has it: the shell's native
 * engine owns the microphone for one utterance at a time and reports its
 * level and words. A call that uses it sends the desk finished text, not
 * audio (see call.ts).
 */

export type TranscriptionCapability = {
	available: boolean;
	onDevice: true;
	engine?: "apple-analyzer" | "apple-recognizer";
	locale: string;
	modelInstalled?: boolean;
	reason?: string;
};

/** Every text event is the whole current utterance, never a delta to append. */
export type TranscriptionEvent =
	| { type: "partial"; text: string }
	| { type: "final"; text: string }
	| { type: "level"; levelDb: number; at: number; unit: "dbfs" }
	| { type: "error"; message: string; code?: string }
	| { type: "ended"; reason: "final" | "no-speech" | "cancelled" | "error" };

export interface DeviceTranscription {
	/** Does not ask permission, download a model, or open the microphone. Never rejects. */
	capability(): Promise<TranscriptionCapability>;
	/** Asks for speech and microphone access, and may download the language. Never rejects. */
	permit(): Promise<boolean>;
	/** Each start is a fresh utterance. False means recognition did not start. */
	start(onEvent: (event: TranscriptionEvent) => void): Promise<boolean>;
	/** Closes the microphone and waits for the complete final text. Never answers a partial on timeout. */
	stop(): Promise<string>;
	/** At once stops every callback, including those of an unfinished start or stop. */
	cancel(): Promise<void>;
}

/** The shell's commands and event, or a fake in tests. */
export type NativeSpeech = {
	capability(): Promise<TranscriptionCapability>;
	permit(): Promise<boolean>;
	start(sessionId: string): Promise<boolean>;
	stop(sessionId: string): Promise<string>;
	cancel(sessionId: string): Promise<void>;
	listen(onEvent: (event: TranscriptionEvent & { sessionId: string }) => void): Promise<() => void>;
};

const tauriSpeech: NativeSpeech = {
	capability: () => invoke<TranscriptionCapability>("speech_capability"),
	permit: () => invoke<boolean>("speech_permit"),
	start: (sessionId) => invoke<boolean>("speech_start", { sessionId }),
	stop: (sessionId) => invoke<string>("speech_stop", { sessionId }),
	cancel: (sessionId) => invoke<void>("speech_cancel", { sessionId }),
	listen: (onEvent) => listen<TranscriptionEvent & { sessionId: string }>("speech-event", (event) => onEvent(event.payload)),
};

/** How long the engine has to finish an utterance once its microphone is closed. */
const FINAL_PATIENCE_MS = 6_000;

/** Only the macOS shell has the engine; a browser tab and the other systems have none. */
function hasNativeSpeech(): boolean {
	return typeof window !== "undefined" && platform() === "macos";
}

export function deviceTranscription(
	module: NativeSpeech | undefined = hasNativeSpeech() ? tauriSpeech : undefined,
	patienceMs = FINAL_PATIENCE_MS,
): DeviceTranscription | undefined {
	if (module === undefined) return undefined;
	let generation = 0;
	let session: string | null = null;
	let unlisten: (() => void) | null = null;
	const prefix = `speech-${Date.now()}-${Math.random().toString(36).slice(2)}`;

	const cancel = async () => {
		generation++;
		const current = session;
		session = null;
		unlisten?.();
		unlisten = null;
		// The empty id also cancels a model download that permit started before any session.
		await module.cancel(current ?? "");
	};

	return {
		// A shell without the engine's commands (an older build) has no speech, rather than an error.
		capability: () =>
			module.capability().catch((error: unknown) => ({
				available: false,
				onDevice: true as const,
				locale: "",
				reason: error instanceof Error ? error.message : String(error),
			})),
		permit: () => module.permit().then((granted) => granted === true, () => false),
		async start(onEvent) {
			const token = ++generation;
			const previous = session;
			session = null;
			unlisten?.();
			unlisten = null;
			if (previous !== null) await module.cancel(previous).catch(() => {});
			if (generation !== token) return false;
			const id = `${prefix}-${token}`;
			// Listen before starting, so the first level is not missed.
			const stopListening = await module.listen((event) => {
				if (generation !== token || event.sessionId !== id) return;
				if (event.type === "level" && (!Number.isFinite(event.levelDb) || !Number.isFinite(event.at))) return;
				const { sessionId: _sessionId, ...value } = event;
				onEvent(value as TranscriptionEvent);
			});
			if (generation !== token) {
				stopListening();
				return false;
			}
			session = id;
			unlisten = stopListening;
			try {
				const started = await module.start(id);
				if (generation !== token) {
					await module.cancel(id);
					return false;
				}
				if (!started) await cancel();
				return started;
			} catch (error) {
				if (generation !== token) return false;
				await cancel().catch(() => {});
				throw error;
			}
		},
		async stop() {
			const id = session;
			const token = generation;
			if (id === null) return "";
			let timer: ReturnType<typeof setTimeout> | undefined;
			try {
				const text = await Promise.race([
					module.stop(id),
					new Promise<never>((_, reject) => {
						timer = setTimeout(() => reject(new Error("Speech recognition on this Mac did not finish. Try again.")), patienceMs);
					}),
				]);
				if (generation !== token) return "";
				unlisten?.();
				unlisten = null;
				session = null;
				return text.trim();
			} catch (error) {
				if (generation !== token) return "";
				await cancel().catch(() => {});
				throw error;
			} finally {
				clearTimeout(timer);
			}
		},
		cancel,
	};
}

/** Whether this machine hears speech itself; asked once, since it does not change while the app runs. */
let hearing: Promise<boolean> | null = null;
export function hearsOnThisMac(): Promise<boolean> {
	hearing ??= (async () => (await deviceTranscription()?.capability())?.available === true)().catch(() => false);
	return hearing;
}

/**
 * This Mac's engine meters its microphone after voice processing, which
 * cancels echo and lowers everything with it: a quiet room measured about
 * -74 dBFS and normal speech -50 to -40, where a raw microphone puts speech
 * near -30. Raised by this much, the call's speech thresholds and the
 * dictation meter, tuned for a raw microphone, hear normal speech again.
 */
export const PROCESSED_GAIN_DB = 24;

/** A level from this Mac's engine as a raw microphone would read it, in dBFS. */
export function rawEquivalentDb(db: number): number {
	return db + PROCESSED_GAIN_DB;
}
