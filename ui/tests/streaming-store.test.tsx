import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { StreamDelta, TranscriptEvent } from "../src/generated/contract";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

let act: typeof import("react").act;
let useSyncExternalStore: typeof import("react").useSyncExternalStore;
let Transcript: typeof import("../src/components/Transcript").Transcript;
let ThreadStore: typeof import("../src/tape").ThreadStore;
let store: InstanceType<typeof ThreadStore>;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let frames: Map<number, FrameRequestCallback>;
let handlers: { snapshot(items: TranscriptEvent[]): void; event(item: TranscriptEvent): void; ephemeral(delta: StreamDelta): void };
const thread = { kind: "dm" as const, key: "streaming-test" };

beforeEach(async () => {
	frames = new Map();
	let frameId = 0;
	const installed = installDom({
		requestAnimationFrame: (callback: FrameRequestCallback) => { frames.set(++frameId, callback); return frameId; },
		cancelAnimationFrame: (id: number) => frames.delete(id),
	});
	teardown = installed.teardown;
	Object.assign(globalThis, { ResizeObserver: installed.dom.ResizeObserver, MutationObserver: installed.dom.MutationObserver, CSS: installed.dom.CSS });
	({ act, useSyncExternalStore } = await import("react"));
	({ ThreadStore } = await import("../src/tape"));
	({ Transcript } = await import("../src/components/Transcript"));
	const wire = {
		onConnection(callback: (state: string) => void) { callback("open"); return () => {}; },
		subscribe(_target: unknown, next: typeof handlers) { handlers = next; return () => {}; },
	} as unknown as Wire;
	store = new ThreadStore(wire, thread);
	store.acquire();
	handlers.snapshot([{ kind: "user", id: "u", ts: 1, text: "Write it" }]);
	container = document.createElement("div");
	root = (await import("react-dom/client")).createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); await teardown(); });

function words(messageId: string, text: string, kind: "text" | "thought" = "text") {
	handlers.ephemeral({ type: "thread_delta", thread, messageId, text, kind });
}
function flush() {
	const due = [...frames.values()];
	frames.clear();
	for (const callback of due) callback(0);
}

test("a long unfinished reply wakes readers once, then publishes the final bubble", async () => {
	let renders = 0;
	let notifications = 0;
	store.subscribe(() => notifications++);
	function Reader() {
		const state = useSyncExternalStore(store.subscribe, store.snapshot);
		renders++;
		return <Transcript personaId="streaming-test" name="Test" events={state.events} streaming={state.streaming} live focus={null} />;
	}
	await act(async () => root.render(<Reader />));
	renders = 0;
	const chunk = "A long reply still being written without a paragraph break. ";
	const started = performance.now();
	for (let index = 0; index < 1000; index++) {
		await act(async () => { words("reply", chunk); flush(); });
	}
	const streamingRenders = renders;
	const streamingNotifications = notifications;
	await act(async () => handlers.event({ kind: "agent", id: "reply", ts: 2, text: chunk.repeat(1000) }));
	console.log(`streaming measurement: ${chunk.length * 1000} chars, 1000 frames, ${streamingNotifications} notifications, ${streamingRenders} reader renders, ${(performance.now() - started).toFixed(1)} ms`);
	expect(streamingNotifications).toBe(1);
	expect(streamingRenders).toBe(1);
	expect(store.snapshot().streaming).toEqual([]);
	expect(store.snapshot().events.at(-1)).toMatchObject({ id: "reply", text: chunk.repeat(1000) });
	expect(container.querySelector('[data-event-id="reply"]')?.textContent).toContain(chunk.repeat(1000).trim());
});

test("whole bubbles retain their stamp, thoughts grow, and queued words settle atomically with tools and cards", () => {
	const paragraph = "A complete paragraph with enough words to stand on its own without joining the next bubble.";
	words("reply", `${paragraph}\n\n`);
	flush();
	const first = store.snapshot();
	words("reply", "\n");
	flush();
	expect(store.snapshot()).toBe(first);
	words("reply", "Still writing");
	flush();
	expect(store.snapshot()).toBe(first);
	expect(first.streaming[0]?.ts).toBeNumber();
	words("thought", "First thought", "thought");
	flush();
	words("thought", " continued", "thought");
	flush();
	expect(store.snapshot().streaming[1]?.text).toBe("First thought continued");
	expect(store.snapshot().streaming[0]).toBe(first.streaming[0]);
	let notifications = 0;
	store.subscribe(() => notifications++);
	words("reply", " and finished.");
	handlers.event({ kind: "agent", id: "reply", ts: 2, text: paragraph });
	expect(notifications).toBe(1);
	expect(frames.size).toBe(0);
	handlers.event({ kind: "agent", id: "reply-2", ts: 3, text: "Still writing and finished." });
	expect(store.snapshot().events.filter((one) => one.kind === "agent").map((one) => one.text)).toEqual([paragraph, "Still writing and finished."]);
	expect(store.snapshot().streaming.map((one) => one.messageId)).toEqual(["thought"]);
	const tool = { kind: "tool" as const, id: "tool", toolCallId: "call", ts: 4, title: "Read", status: "pending" as const };
	handlers.event(tool);
	handlers.event({ ...tool, status: "completed", output: [{ type: "text", text: "Found it" }] });
	const card = { kind: "permission" as const, id: "card", requestId: "request", ts: 5, title: "Allow?", options: [] };
	handlers.event(card);
	expect(store.snapshot().events.at(-2)).toMatchObject({ status: "completed", output: [{ type: "text", text: "Found it" }] });
	expect(store.snapshot().events.at(-1)).toBe(card);
	handlers.snapshot([{ kind: "agent", id: "reply", ts: 2, text: "Restored reply" }]);
	expect(store.snapshot().streaming).toEqual([]);
	expect(store.snapshot().events).toHaveLength(1);
});

test("tool results and answered cards update the rendered transcript during a reply", async () => {
	function Reader() {
		const state = useSyncExternalStore(store.subscribe, store.snapshot);
		return <Transcript personaId="streaming-test" name="Test" thread={{ kind: "call", key: "call" }} events={state.events} streaming={state.streaming} live focus={null} />;
	}
	await act(async () => root.render(<Reader />));
	const tool = { kind: "tool" as const, id: "tool", toolCallId: "call", ts: 2, title: "Read file", status: "pending" as const };
	const card = { kind: "permission" as const, id: "card", requestId: "request", ts: 3, title: "Allow the read?", options: [{ optionId: "yes", name: "Allow once" }] };
	await act(async () => {
		words("reply", "Still unfinished");
		flush();
		handlers.event(tool);
		handlers.event(card);
	});
	expect(container.querySelector('[data-step-id="tool"]')?.textContent).toContain("pending");
	expect(container.querySelector('[data-event-id="card"] button')?.textContent).toBe("Allow once");
	await act(async () => {
		handlers.event({ ...tool, status: "completed", output: [{ type: "text", text: "The result is here" }] });
		handlers.event({ ...card, decision: "yes" });
	});
	expect(container.querySelector('[data-step-id="tool"]')?.textContent).toContain("The result is here");
	expect(container.querySelector('[data-event-id="card"] button')).toBeNull();
	expect(container.querySelector('[data-event-id="card"]')?.textContent).toContain("Asked permission");
});
