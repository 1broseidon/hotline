import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import { readFileSync } from "node:fs";
import type { Root } from "react-dom/client";
import type { CapabilityOptions, Credential, Provider, SpeechModel } from "../src/generated/contract";
import type { Handlers } from "../src/wire";
import { installDom } from "./dom";

const observer = { ResizeObserver: class { observe() {} unobserve() {} disconnect() {} } };
const boot = installDom(observer);
const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { Settings, SettingsRail } = await import("../src/components/Settings");
const { replaceDesks } = await import("../src/desks");
const { DICTATION_UNAVAILABLE } = await import("../src/voice/dictation");
const { Wire, wire } = await import("../src/wire");
await boot.teardown();

const DESK = { providerId: "local", providerName: "On the desk", onDevice: true, cost: "free" };

/** The desk's catalogue: Parakeet and the desk's voice installed, Whisper offered. */
function catalogue(): SpeechModel[] {
	const base = { credit: "", licenceUrl: "https://example.com/licence", downloadBytes: 0, diskBytes: 0 };
	return [
		{ ...base, id: "parakeet-tdt-0.6b-v3", name: "Parakeet", detail: "25 European languages", diskBytes: 670_000_000, credit: "NVIDIA Parakeet TDT 0.6B v3, CC BY 4.0", state: "installed" },
		{ ...base, id: "whisper-large-v3-turbo", name: "Whisper", detail: "99 languages, slower", downloadBytes: 1_040_000_000, state: "available" },
		{ ...base, id: "phonon-31m-en", name: "Phonon", detail: "Speaks English", diskBytes: 66_000_000, credit: "Gradium Phonon 31M English, Apache 2.0", state: "installed" },
	];
}

function capabilities(): CapabilityOptions {
	return {
		images: { options: [{ providerId: "openai", providerName: "OpenAI", models: [{ id: "gpt-image-1" }] }] },
		stt: {
			automatic: { providerId: "local", providerName: "On the desk", modelId: "parakeet-tdt-0.6b-v3" },
			options: [{ ...DESK, models: [{ id: "parakeet-tdt-0.6b-v3", label: "Parakeet TDT 0.6B" }] }],
		},
		tts: {
			automatic: { providerId: "local", providerName: "On the desk", modelId: "phonon-31m-en" },
			options: [{ ...DESK, models: [{ id: "phonon-31m-en", label: "Phonon", voices: ["Edwin"] }] }],
		},
		dispatcher: { options: [{ providerId: "openai", providerName: "OpenAI", models: [{ id: "gpt-5-mini" }] }] },
		spending: { budgets: [] },
		deviceName: "this computer",
	};
}

const credential: Credential = { id: "one", providerId: "openai", credentialKind: "api_key", label: "OpenAI", revoked: false, createdAt: 1, updatedAt: 1 };
const providers: Provider[] = [{ id: "openai", name: "OpenAI", credentialKinds: ["api_key"], modelDiscovery: false, does: ["chat"] }];

let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let desks = 0;
let models: SpeechModel[];
let asked: { cmd: string; params: unknown }[];

beforeEach(() => {
	({ teardown } = installDom(observer));
	models = catalogue();
	asked = [];
	spyOn(Wire.prototype, "subscribe").mockImplementation(((target: unknown, handlers: Handlers<unknown, unknown>) => {
		if (target === "room") handlers.snapshot([{ kind: "setting", id: "voice", value: { front: { provider: "turn-by-turn" }, listenFor: ["Ophelia"] } }]);
		return () => {};
	}) as InstanceType<typeof Wire>["subscribe"]);
	spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		asked.push({ cmd, params });
		const id = (params as { modelId?: string }).modelId;
		if (cmd === "voice.model_install") models = models.map((one) => (one.id === id ? { ...one, state: "downloading", receivedBytes: 0 } : one));
		if (cmd === "voice.model_cancel") models = models.map((one) => (one.id === id ? { ...one, state: "available" } : one));
		if (cmd === "voice.model_remove") models = models.map((one) => (one.id === id ? { ...one, state: "available" } : one));
		if (cmd.startsWith("voice.model")) return models;
		if (cmd === "capabilities.options") return capabilities();
		if (cmd === "credential.list") return [credential];
		if (cmd === "providers.list") return providers;
		return {};
	}) as typeof wire.command);
	// A desk of its own for each test, so each starts from its own room.
	replaceDesks([{ id: `settings-voice-${++desks}`, kind: "local", name: "Test", origin: "ws://localhost", token: "test" }]);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	replaceDesks([]);
	await teardown();
});

async function draw(node: React.ReactNode) {
	await act(async () => root.render(node));
	await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
}
const settle = () => act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
const heading = () => container.querySelector("h2")?.textContent;
const groupTitles = () => [...container.querySelectorAll(".group-title")].map((node) => node.textContent);
/** Each row's title without the tag beside it, and each fold's title. */
const rows = () => [...container.querySelectorAll(".group-row-title, .nt-fold-title")].map((node) => node.firstChild?.textContent);
const button = (name: string) =>
	[...container.querySelectorAll<HTMLButtonElement>("button")].find((node) => node.getAttribute("aria-label") === name || (node.textContent ?? "").startsWith(name));

test("the rail lists Voice directly after Providers", async () => {
	await draw(<SettingsRail section="voice" onSection={() => {}} onBack={() => {}} />);
	const names = [...container.querySelectorAll(".rail-row")].map((node) => node.textContent);
	expect(names.slice(names.indexOf("Providers"), names.indexOf("Providers") + 2)).toEqual(["Providers", "Voice"]);
	expect(container.querySelector('.rail-row[aria-current="true"]')?.textContent).toBe("Voice");
});

test("Voice holds every call setting in today's order as one group, then one Voice models row", async () => {
	await draw(<Settings section="voice" />);
	expect(heading()).toBe("Voice");
	expect(groupTitles()).toEqual(["Calls"]);
	expect(rows()).toEqual(["Model", "Transcription", "Speech", "Call assistant", "Acknowledge while the teammate thinks", "Words to listen for", "Voice models"]);
	// The calls are one card; the models row is a card of its own that keeps its frame alone.
	expect(container.querySelectorAll(".grouped").length).toBe(2);
	const row = button("Voice models")!;
	expect(row.classList.contains("group-row-page")).toBe(true);
	expect(row.textContent).toBe("Voice models2 installed · 736 MB");
	expect(container.textContent).not.toContain("Models on the desk");
});

test("the models page lists the desk's voice under Speech and Parakeet under Transcription, and goes back to Voice", async () => {
	await draw(<Settings section="voice" />);
	await act(async () => button("Voice models")!.click());
	expect(heading()).toBe("Voice models");
	expect(groupTitles()).toEqual(["Installed", "Recommended"]);
	expect(container.textContent).toContain("736 MB used");
	const tagOf = (name: string) =>
		[...container.querySelectorAll(".group-row-title")].find((node) => node.firstChild?.textContent === name)?.querySelector(".model-kind")?.textContent;
	expect(tagOf("Phonon")).toBe("Speech");
	expect(tagOf("Parakeet")).toBe("Transcription");
	expect(tagOf("Whisper")).toBe("Transcription");
	const details = [...container.querySelectorAll(".group-row-detail")].map((node) => node.textContent);
	expect(details).toEqual(["670 MB · 25 European languages · Used for calls", "66 MB · Speaks English · Used for calls", "99 languages, slower"]);
	// Credits wait behind a button on each installed model.
	expect(container.textContent).not.toContain("NVIDIA Parakeet TDT 0.6B v3, CC BY 4.0");
	await act(async () => button("Credits for Parakeet")!.click());
	expect(container.textContent).toContain("NVIDIA Parakeet TDT 0.6B v3, CC BY 4.0");
	await act(async () => button("Back to the team")!.click());
	expect(heading()).toBe("Voice");
});

test("download, cancel and remove work from the models page, and the row follows", async () => {
	await draw(<Settings section="voice" />);
	await act(async () => button("Voice models")!.click());
	await act(async () => button("Download 1.0 GB")!.click());
	await settle();
	expect(asked.at(-1)).toEqual({ cmd: "voice.model_install", params: { modelId: "whisper-large-v3-turbo" } });
	expect(container.querySelector('[role="progressbar"]')?.getAttribute("aria-label")).toBe("Downloading Whisper");
	expect(container.textContent).toContain("99 languages, slower · Downloading 1 MB of 1.0 GB");
	await act(async () => button("Stop downloading Whisper")!.click());
	await settle();
	expect(asked.at(-1)).toEqual({ cmd: "voice.model_cancel", params: { modelId: "whisper-large-v3-turbo" } });
	expect(container.querySelector('[role="progressbar"]')).toBeNull();
	const before = asked.filter((one) => one.cmd === "capabilities.options").length;
	await act(async () => button("Remove Phonon")!.click());
	await settle();
	expect(asked.filter((one) => one.cmd.startsWith("voice.model_")).at(-1)).toEqual({ cmd: "voice.model_remove", params: { modelId: "phonon-31m-en" } });
	// What is installed changed, so what can speak is asked again; the removed voice is offered again, still as Speech.
	expect(asked.filter((one) => one.cmd === "capabilities.options").length).toBe(before + 1);
	const recommended = [...container.querySelectorAll("section")].find((node) => node.querySelector(".group-title")?.textContent === "Recommended")!;
	expect(recommended.textContent).toContain("PhononSpeech");
	await act(async () => button("Back to the team")!.click());
	expect(button("Voice models")!.textContent).toBe("Voice models1 installed · 670 MB");
});

test("Providers keeps its accounts, Images and Budgets, and nothing about calls or the desk's models", async () => {
	await draw(<Settings section="providers" />);
	expect(heading()).toBe("Providers");
	expect(groupTitles()).toContain("Use for");
	expect(rows()).toContain("OpenAI");
	expect(rows()).toContain("Images");
	for (const gone of ["Calls", "Transcription", "Speech", "Call assistant", "Acknowledge while the teammate thinks", "Words to listen for", "Models on the desk", "Voice models"]) {
		expect(container.textContent).not.toContain(gone);
	}
	expect(asked.some((one) => one.cmd === "voice.models")).toBe(false);
});

test("what pointed at the call settings under Providers points at Voice", () => {
	expect(DICTATION_UNAVAILABLE).toBe("Dictation isn't available yet. Download a speech model for the desk in Settings › Voice.");
});

test("a lone row keeps its card only when it opens a page", () => {
	const css = readFileSync(new URL("../src/index.css", import.meta.url), "utf8");
	expect(css).toContain(".pane-column .grouped:has(> .group-row:only-child:not(.group-row-page))");
});
