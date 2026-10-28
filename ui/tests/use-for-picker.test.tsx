import { expect, spyOn, test } from "bun:test";
import type { CapabilityOptions } from "../src/generated/contract";
import { installDom } from "./dom";

test("ChatGPT images require a picker selection, Automatic clears it, and a chosen live model that is not connected says so", async () => {
	const { teardown } = installDom();
	// React probes event support at module load; install the DOM before loading it.
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { UseFor } = await import("../src/components/UseFor");
	const { wire } = await import("../src/wire");
	const calls: { cmd: string; params: unknown }[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		calls.push({ cmd, params });
		return {};
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const provider = { providerId: "openai-codex", providerName: "Codex (ChatGPT subscription)" };
	const options: CapabilityOptions = {
		images: { options: [{ ...provider, models: [{ id: "gpt-image-2" }] }] },
		stt: { options: [] },
		tts: { options: [] },
		dispatcher: { options: [] },
		spending: { budgets: [] },
	};
	let changes = 0;
	const render = () => act(async () => { root.render(<UseFor options={options} voice={undefined} onChanged={() => { changes += 1; }} />); });
	const pick = async (name: string) => {
		await act(async () => { (container.querySelector('[aria-label="Model for images"]') as HTMLButtonElement).click(); });
		expect(document.body.textContent).toContain("Codex (ChatGPT subscription)");
		const item = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((node) => node.textContent === name);
		expect(item).toBeDefined();
		await act(async () => { item!.click(); });
	};
	try {
		await render();
		expect(calls).toEqual([]);
		expect(container.textContent).toContain("Choose a provider for images");
		await pick("gpt-image-2");
		expect(calls).toEqual([{ cmd: "settings.update", params: { patch: { images: { provider: "openai-codex", model: "gpt-image-2" } } } }]);
		options.images.selected = { ...provider, modelId: "gpt-image-2" };
		await render();
		expect(container.textContent).toContain("Codex (ChatGPT subscription) · gpt-image-2");
		await pick("Automatic");
		expect(calls.at(-1)).toEqual({ cmd: "settings.update", params: { patch: { images: null } } });
		expect(changes).toBe(2);
		// Calls default to Grok Voice, and nothing connected runs it.
		expect(container.textContent).toContain("Not connected. Connect its provider above, or choose another.");
		expect(container.textContent).not.toContain("Transcription");
	} finally {
		await act(async () => { root.unmount(); });
		command.mockRestore();
		await teardown();
	}
});

test("each call model shows only its own rows: a live model its voice, Turn by turn Transcription and Speech, and a saved Legacy choice reads as Turn by turn", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { UseFor } = await import("../src/components/UseFor");
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const options: CapabilityOptions = {
		images: { options: [] },
		stt: {
			selected: { providerId: "local", providerName: "On the desk", modelId: "parakeet-tdt-0.6b-v3" },
			options: [{ providerId: "local", providerName: "On the desk", onDevice: true, cost: "free", models: [{ id: "parakeet-tdt-0.6b-v3", label: "Parakeet TDT 0.6B" }] }],
		},
		tts: {
			selected: { providerId: "ollama", providerName: "Ollama", modelId: "phonon", voice: "Zoey" },
			options: [{ providerId: "ollama", providerName: "Ollama", onDevice: true, cost: "free", models: [{ id: "phonon", label: "phonon", voices: ["default", "Zoey"] }] }],
		},
		dispatcher: { options: [{ providerId: "xai", providerName: "xAI", models: [{ id: "grok-4.7" }] }] },
		spending: { budgets: [] },
		live: [{ front: "grok", credential: "xai-subscription", modelId: "grok-voice-latest", name: "Grok Voice", providerName: "Grok subscription", voices: ["eve", "ara"] }],
		deviceName: "this computer",
	};
	const rows = () => [...container.querySelectorAll(".group-row-title")].map((node) => node.firstChild?.textContent);
	const render = (voice: unknown) => act(async () => root.render(<UseFor options={options} voice={voice} onChanged={() => {}} />));
	try {
		await render({ front: { provider: "grok", credential: "xai-subscription" } });
		expect(rows()).toEqual(["Images", "Model", "Voice", "Call assistant"]);
		expect(container.textContent).toContain("Hears, talks and uses tools in one stream");
		expect(container.textContent).toContain("Eve on Grok subscription");
		expect(container.textContent).toContain("Talk over it to interrupt.");

		for (const provider of ["turn-by-turn", "legacy"]) {
			await render({ front: { provider }, stt: { provider: "local", model: "parakeet-tdt-0.6b-v3" }, tts: { provider: "ollama", model: "phonon", voice: "Zoey" } });
			expect(rows()).toEqual(["Images", "Model", "Transcription", "Speech", "Call assistant"]);
			expect(container.textContent).toContain("The teammate answers; the desk transcribes and speaks");
			expect(container.textContent).toContain("Parakeet · free");
			expect(container.textContent).toContain("Phonon through Ollama · free");
			expect(container.textContent).toContain("Phonon · Zoey");
			expect([...container.querySelectorAll(".use-for-tag")].map((node) => node.textContent)).toEqual(["on this computer", "on this computer"]);
			expect(container.textContent).toContain("Tap to interrupt.");
			expect(container.textContent).not.toMatch(/legacy/i);
		}
	} finally {
		await act(async () => root.unmount());
		await teardown();
	}
});
