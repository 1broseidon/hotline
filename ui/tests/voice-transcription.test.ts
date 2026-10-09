import { describe, expect, test } from "bun:test";
import { type NativeSpeech, type TranscriptionEvent, deviceTranscription } from "../src/voice/transcription";

type Sent = TranscriptionEvent & { sessionId: string };

function fakeNative(overrides: Partial<NativeSpeech> = {}) {
	const calls: string[] = [];
	const listeners = new Set<(event: Sent) => void>();
	let finals: (sessionId: string) => Promise<string> = async () => "";
	const started: string[] = [];
	const native: NativeSpeech = {
		capability: async () => ({ available: true, onDevice: true, locale: "en-US", engine: "apple-analyzer" }),
		permit: async () => true,
		start: async (sessionId) => {
			calls.push(`start ${sessionId}`);
			started.push(sessionId);
			return true;
		},
		stop: (sessionId) => {
			calls.push(`stop ${sessionId}`);
			return finals(sessionId);
		},
		cancel: async (sessionId) => {
			calls.push(`cancel ${sessionId}`);
		},
		listen: async (onEvent) => {
			listeners.add(onEvent);
			return () => listeners.delete(onEvent);
		},
		...overrides,
	};
	return {
		native,
		calls,
		listeners,
		session: () => started.at(-1)!,
		emit: (event: Sent) => {
			for (const listener of [...listeners]) listener(event);
		},
		finals: (next: (sessionId: string) => Promise<string>) => (finals = next),
	};
}

describe("speech recognition on this Mac", () => {
	test("there is none outside the macOS shell", () => {
		expect(deviceTranscription()).toBeUndefined();
	});

	test("a shell without the commands has no speech, rather than an error", async () => {
		const fake = fakeNative({
			capability: () => Promise.reject(new Error("command speech_capability not found")),
			permit: () => Promise.reject(new Error("command speech_permit not found")),
		});
		const speech = deviceTranscription(fake.native)!;
		const capability = await speech.capability();
		expect(capability.available).toBe(false);
		expect(capability.reason).toContain("speech_capability");
		expect(await speech.permit()).toBe(false);
	});

	test("only the open session's events arrive, without their session id, and a broken level is dropped", async () => {
		const fake = fakeNative();
		const speech = deviceTranscription(fake.native)!;
		const heard: TranscriptionEvent[] = [];
		expect(await speech.start((event) => heard.push(event))).toBe(true);
		const id = fake.session();
		fake.emit({ sessionId: "someone-else", type: "partial", text: "not mine" });
		fake.emit({ sessionId: id, type: "partial", text: "call" });
		fake.emit({ sessionId: id, type: "level", levelDb: Number.NaN, at: 1, unit: "dbfs" });
		fake.emit({ sessionId: id, type: "level", levelDb: -20, at: 2, unit: "dbfs" });
		expect(heard).toEqual([{ type: "partial", text: "call" }, { type: "level", levelDb: -20, at: 2, unit: "dbfs" }]);
	});

	test("each start is a fresh session that lets the last one go", async () => {
		const fake = fakeNative();
		const speech = deviceTranscription(fake.native)!;
		const first: TranscriptionEvent[] = [];
		await speech.start((event) => first.push(event));
		const one = fake.session();
		await speech.start(() => {});
		const two = fake.session();
		expect(two).not.toBe(one);
		expect(fake.calls).toEqual([`start ${one}`, `cancel ${one}`, `start ${two}`]);
		fake.emit({ sessionId: one, type: "partial", text: "late" });
		expect(first).toEqual([]);
		expect(fake.listeners.size).toBe(1);
	});

	test("stop answers the complete final text and closes the session", async () => {
		const fake = fakeNative();
		fake.finals(async () => "  Call Mack.  ");
		const speech = deviceTranscription(fake.native)!;
		await speech.start(() => {});
		expect(await speech.stop()).toBe("Call Mack.");
		expect(fake.listeners.size).toBe(0);
		// Nothing is open now, so there is nothing to stop.
		expect(await speech.stop()).toBe("");
	});

	test("an engine that never finishes is given up on, never answered with a partial", async () => {
		const fake = fakeNative();
		fake.finals(() => new Promise(() => {}));
		const speech = deviceTranscription(fake.native, 20)!;
		await speech.start(() => {});
		const id = fake.session();
		await expect(speech.stop()).rejects.toThrow("did not finish");
		expect(fake.calls.at(-1)).toBe(`cancel ${id}`);
		expect(fake.listeners.size).toBe(0);
	});

	test("cancel during stop answers nothing, even when the final arrives after", async () => {
		const fake = fakeNative();
		let finish: (text: string) => void = () => {};
		fake.finals(() => new Promise((resolve) => (finish = resolve)));
		const speech = deviceTranscription(fake.native)!;
		await speech.start(() => {});
		const stopping = speech.stop();
		await speech.cancel();
		finish("words from a turn that was held");
		expect(await stopping).toBe("");
	});

	test("cancel before a start has listened means the engine never starts", async () => {
		let release: () => void = () => {};
		const fake = fakeNative();
		const listen = fake.native.listen;
		fake.native.listen = async (onEvent) => {
			await new Promise<void>((resolve) => (release = resolve));
			return listen(onEvent);
		};
		const speech = deviceTranscription(fake.native)!;
		const starting = speech.start(() => {});
		await Promise.resolve();
		await speech.cancel();
		release();
		expect(await starting).toBe(false);
		expect(fake.calls.some((call) => call.startsWith("start"))).toBe(false);
		expect(fake.listeners.size).toBe(0);
	});

	test("an engine that will not start is let go", async () => {
		const fake = fakeNative({ start: async () => false });
		const speech = deviceTranscription(fake.native)!;
		expect(await speech.start(() => {})).toBe(false);
		expect(fake.calls.at(-1)?.startsWith("cancel")).toBe(true);
		expect(fake.listeners.size).toBe(0);
	});
});
