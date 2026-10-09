import { describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import type { CapabilityOptions, SpeechModel } from "../src/generated/contract";
import { busy, installedChanged, megabytes, modelLine, progress } from "../src/voice/deskModels";

function model(overrides: Partial<SpeechModel> = {}): SpeechModel {
	return {
		id: "parakeet-tdt-110m-en",
		name: "Parakeet English",
		detail: "English only, small and quick",
		downloadBytes: 108_035_095,
		diskBytes: 136_490_421,
		credit: "NVIDIA Parakeet TDT 110M, CC BY 4.0, quantized by sherpa-onnx",
		licenceUrl: "https://creativecommons.org/licenses/by/4.0/",
		state: "available",
		...overrides,
	};
}

describe("the desk's speech models in words", () => {
	test("sizes read as a download dialog says them", () => {
		expect(megabytes(108_035_095)).toBe("108 MB");
		expect(megabytes(487_170_055)).toBe("487 MB");
		expect(megabytes(1_200_000_000)).toBe("1.2 GB");
		expect(megabytes(10)).toBe("1 MB");
	});

	test("each state says where the model stands", () => {
		expect(modelLine(model())).toBe("English only, small and quick · 108 MB download");
		expect(modelLine(model({ state: "downloading", receivedBytes: 54_000_000 }))).toBe("Downloading 54 MB of 108 MB");
		expect(progress(model({ state: "downloading", receivedBytes: 54_017_547 }))).toBeCloseTo(0.5, 3);
		expect(modelLine(model({ state: "unpacking" }))).toBe("Unpacking");
		expect(modelLine(model({ state: "installed" }))).toBe("English only, small and quick · 136 MB on the desk");
		// A model the catalogue no longer lists has nothing more to say.
		expect(modelLine(model({ state: "installed", detail: "", diskBytes: 0 }))).toBe("");
	});

	test("only a download under way is asked after, and only a change in what is installed refreshes hearing", () => {
		expect(busy([model(), model({ state: "installed" })])).toBe(false);
		expect(busy([model(), model({ state: "downloading" })])).toBe(true);
		expect(busy([model({ state: "unpacking" })])).toBe(true);
		const before = [model({ state: "downloading" })];
		expect(installedChanged(before, [model({ state: "unpacking" })])).toBe(false);
		expect(installedChanged(before, [model({ state: "installed" })])).toBe(true);
		expect(installedChanged([model({ state: "installed" })], [model()])).toBe(true);
	});
});

test("a model is downloaded only when asked, followed until it lands, and removed; Voice opens even with nothing connected", async () => {
	const dom = new Window();
	const restores: (() => void)[] = [];
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		});
	}
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { UseFor } = await import("../src/components/UseFor");
	const { wire } = await import("../src/wire");
	// What the desk answers next, as the test moves it along.
	let models: SpeechModel[] = [model({ id: "parakeet-tdt-0.6b-v3", name: "Parakeet", downloadBytes: 487_170_055 }), model()];
	const asked: { cmd: string; params: unknown }[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		asked.push({ cmd, params });
		if (cmd === "voice.model_install") models = models.map((one) => (one.id === (params as { modelId: string }).modelId ? { ...one, state: "downloading", receivedBytes: 0 } : one));
		if (cmd === "voice.model_remove") models = models.map((one) => ({ ...one, state: "available" }));
		if (cmd.startsWith("voice.model")) return models;
		return {};
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	// Nothing connected: nothing can speak or hear, and the desk has no model yet.
	const options: CapabilityOptions = {
		images: { options: [] },
		stt: { options: [], unavailable: "Download a speech model for the desk, or connect OpenAI, Google, OpenRouter, Groq or Mistral to hear you." },
		tts: { options: [], unavailable: "Connect OpenAI, Google, OpenRouter or Groq to speak." },
		dispatcher: { options: [] },
		spending: { budgets: [] },
	};
	let refreshed = 0;
	const settle = () => act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
	const button = (label: string) =>
		[...container.querySelectorAll<HTMLButtonElement>("button")].find((node) => node.textContent === label || node.getAttribute("aria-label") === label);
	try {
		await act(async () => { root.render(<UseFor options={options} voice={undefined} onChanged={() => { refreshed += 1; }} />); });
		expect(asked).toEqual([]);
		await act(async () => { button("Transcription")!.click(); });
		await settle();
		expect(asked.map((one) => one.cmd)).toEqual(["voice.models"]);
		expect(container.textContent).toContain("Download a speech model for the desk");
		expect(container.textContent).toContain("Parakeet English");
		expect(container.textContent).toContain("NVIDIA Parakeet TDT 110M, CC BY 4.0");

		await act(async () => { button("Download 108 MB")!.click(); });
		await settle();
		expect(asked.at(-1)).toEqual({ cmd: "voice.model_install", params: { modelId: "parakeet-tdt-110m-en" } });
		expect(container.querySelector('[role="progressbar"]')?.getAttribute("aria-valuenow")).toBe("0");
		expect(button("Stop downloading Parakeet English")).toBeDefined();
		// The bigger model was not asked for, and is not coming.
		expect(button("Download 487 MB")).toBeDefined();

		// The window asks again while it downloads; the desk says half, then done.
		models = models.map((one) => (one.state === "downloading" ? { ...one, receivedBytes: 54_017_547 } : one));
		await act(async () => { await new Promise((resolve) => setTimeout(resolve, 600)); });
		expect(container.querySelector('[role="progressbar"]')?.getAttribute("aria-valuenow")).toBe("50");
		expect(refreshed).toBe(0);
		models = models.map((one) => (one.state === "downloading" ? { ...one, state: "installed", receivedBytes: undefined } : one));
		await act(async () => { await new Promise((resolve) => setTimeout(resolve, 600)); });
		expect(container.querySelector('[role="progressbar"]')).toBeNull();
		expect(container.textContent).toContain("136 MB on the desk");
		expect(refreshed).toBe(1);

		await act(async () => { button("Remove Parakeet English")!.click(); });
		await settle();
		expect(asked.at(-1)).toEqual({ cmd: "voice.model_remove", params: { modelId: "parakeet-tdt-110m-en" } });
		expect(refreshed).toBe(2);
	} finally {
		await act(async () => { root.unmount(); });
		command.mockRestore();
		for (const restore of restores.reverse()) restore();
		await dom.happyDOM.close();
	}
});
